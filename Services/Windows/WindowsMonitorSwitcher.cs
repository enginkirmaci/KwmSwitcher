using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.Threading.Tasks;
using KwmSwitcher.Models;

namespace KwmSwitcher.Services.Windows;

[SupportedOSPlatform("windows")]
public class WindowsMonitorSwitcher : IMonitorSwitcher
{
    private readonly AppConfig _config;

    public WindowsMonitorSwitcher(AppConfig config)
    {
        _config = config;
    }

    [DllImport("dxva2.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetNumberOfPhysicalMonitorsFromHMONITOR(
        IntPtr hMonitor, ref uint pdwNumberOfPhysicalMonitors);

    [DllImport("dxva2.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetPhysicalMonitorsFromHMONITOR(
        IntPtr hMonitor, uint dwPhysicalMonitorArraySize,
        [Out] PHYSICAL_MONITOR[] pPhysicalMonitorArray);

    [DllImport("dxva2.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyPhysicalMonitors(
        uint dwPhysicalMonitorArraySize, PHYSICAL_MONITOR[] pPhysicalMonitorArray);

    [DllImport("dxva2.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetVCPFeature(
        IntPtr hMonitor, byte bVCPCode, uint dwVCPFeatureValue);

    [DllImport("dxva2.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetVCPFeatureAndVCPFeatureReply(
        IntPtr hMonitor, byte bVCPCode, out uint pvct, out uint pdwCurrentValue, out uint pdwMaximumValue);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct PHYSICAL_MONITOR
    {
        public IntPtr hPhysicalMonitor;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string szPhysicalMonitorDescription;
    }

    [DllImport("user32.dll")]
    private static extern int EnumDisplayMonitors(
        IntPtr hdc, IntPtr lprcClip, MonitorEnumProc lpfnEnum, IntPtr dwData);

    private delegate bool MonitorEnumProc(
        IntPtr hMonitor, IntPtr hdcMonitor, IntPtr lprcMonitor, IntPtr dwData);

    /// <summary>
    /// Lists every monitor description Windows can identify, and flags any monitor
    /// for which the physical monitor handle came back NULL (meaning a description
    /// is available via EDID, but no DDC/CI control session could be opened, so
    /// SetVCPFeature/GetVCPFeature calls cannot reach that monitor).
    /// </summary>
    public static IReadOnlyList<string> GetAvailableMonitorDescriptions()
    {
        var descriptions = new List<string>();

        foreach (var monitorHandle in GetDisplayMonitorHandles())
        {
            var monitors = GetPhysicalMonitors(monitorHandle);
            if (monitors == null)
                continue;

            try
            {
                foreach (var monitor in monitors)
                {
                    if (string.IsNullOrWhiteSpace(monitor.szPhysicalMonitorDescription))
                        continue;

                    descriptions.Add(monitor.szPhysicalMonitorDescription);

                    if (monitor.hPhysicalMonitor == IntPtr.Zero)
                    {
                        Console.Error.WriteLine(
                            $"[diag] '{monitor.szPhysicalMonitorDescription}' was enumerated with a NULL " +
                            "physical monitor handle. The description came from EDID, but no DDC/CI " +
                            "control session is available for this monitor, so VCP commands cannot be " +
                            "sent to it. This is a driver/connection-level issue (e.g. USB-C dock/KVM/MST " +
                            "hub not passing through the DDC channel, hybrid-GPU routing, or needing to " +
                            "run elevated) rather than something fixable in this code.");
                    }
                }
            }
            finally
            {
                DestroyPhysicalMonitors((uint)monitors.Length, monitors);
            }
        }

        return descriptions;
    }

    public async Task<bool> SetInputSourceAsync(byte inputSource)
    {
        return await Task.Run(() =>
        {
            var vcpCode = MonitorInputSource.GetVcpCode(_config.InputProtocol);
            var value = MonitorInputSource.GetProtocolValue(_config.InputProtocol, inputSource);

            var found = false;

            foreach (var monitor in EnumerateTargetMonitors(logSkipped: true))
            {
                found = true;

                if (SetVCPFeature(monitor.hPhysicalMonitor, vcpCode, value))
                {
                    return true;
                }

                var error = Marshal.GetLastWin32Error();
                Console.Error.WriteLine(
                    $"[diag] SetVCPFeature failed for '{monitor.szPhysicalMonitorDescription}' " +
                    $"(VCP 0x{vcpCode:X2} = 0x{value:X2}). Win32 error code: {error}.");
            }

            if (!found)
            {
                Console.Error.WriteLine(
                    "[diag] No physical monitor with a usable (non-NULL) handle matched the configured " +
                    "target. Call GetAvailableMonitorDescriptions() to see what Windows detected, including " +
                    "any monitors with NULL handles.");
            }

            return false;
        });
    }

    public async Task<byte> GetInputSourceAsync()
    {
        return await Task.Run(() =>
        {
            var vcpCode = MonitorInputSource.GetVcpCode(_config.InputProtocol);

            foreach (var monitor in EnumerateTargetMonitors(logSkipped: true))
            {
                if (GetVCPFeatureAndVCPFeatureReply(
                    monitor.hPhysicalMonitor,
                    vcpCode,
                    out _, out var currentValue, out _))
                {
                    return (byte)currentValue;
                }

                var error = Marshal.GetLastWin32Error();
                Console.Error.WriteLine(
                    $"[diag] GetVCPFeatureAndVCPFeatureReply failed for " +
                    $"'{monitor.szPhysicalMonitorDescription}' (VCP 0x{vcpCode:X2}). " +
                    $"Win32 error code: {error}.");
            }

            return (byte)0;
        });
    }

    private IEnumerable<PHYSICAL_MONITOR> EnumerateTargetMonitors(bool logSkipped = false)
    {
        var targetName = _config.TargetMonitorName;

        foreach (var monitorHandle in GetDisplayMonitorHandles())
        {
            var monitors = GetPhysicalMonitors(monitorHandle);
            if (monitors == null)
                continue;

            try
            {
                foreach (var monitor in monitors)
                {
                    if (!string.IsNullOrWhiteSpace(targetName) &&
                        monitor.szPhysicalMonitorDescription != null &&
                        !monitor.szPhysicalMonitorDescription.Contains(targetName, StringComparison.OrdinalIgnoreCase))
                    {
                        continue;
                    }

                    if (monitor.hPhysicalMonitor == IntPtr.Zero)
                    {
                        if (logSkipped)
                        {
                            Console.Error.WriteLine(
                                $"[diag] Skipping '{monitor.szPhysicalMonitorDescription}': " +
                                "NULL physical monitor handle (no DDC/CI control session).");
                        }
                        continue;
                    }

                    yield return monitor;
                }
            }
            finally
            {
                DestroyPhysicalMonitors((uint)monitors.Length, monitors);
            }
        }
    }

    private static IReadOnlyList<IntPtr> GetDisplayMonitorHandles()
    {
        var handles = new List<IntPtr>();

        EnumDisplayMonitors(
            IntPtr.Zero,
            IntPtr.Zero,
            (hMonitor, _, _, _) =>
            {
                handles.Add(hMonitor);
                return true;
            },
            IntPtr.Zero);

        return handles;
    }

    private static PHYSICAL_MONITOR[]? GetPhysicalMonitors(IntPtr hMonitor)
    {
        uint count = 0;
        if (!GetNumberOfPhysicalMonitorsFromHMONITOR(hMonitor, ref count))
        {
            Console.Error.WriteLine(
                $"[diag] GetNumberOfPhysicalMonitorsFromHMONITOR failed. Win32 error code: " +
                $"{Marshal.GetLastWin32Error()}.");
            return null;
        }

        if (count == 0)
            return null;

        var monitors = new PHYSICAL_MONITOR[count];
        if (!GetPhysicalMonitorsFromHMONITOR(hMonitor, count, monitors))
        {
            Console.Error.WriteLine(
                $"[diag] GetPhysicalMonitorsFromHMONITOR failed. Win32 error code: " +
                $"{Marshal.GetLastWin32Error()}.");
            return null;
        }

        return monitors;
    }
}