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

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern bool EnumDisplayDevices(
        string? lpDevice, uint iDevNum, ref DISPLAY_DEVICE lpDisplayDevice, uint dwFlags);

    private const uint EDS_ATTACHEDTO_DESKTOP = 0x1;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct DISPLAY_DEVICE
    {
        public int cb;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
        public string DeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string DeviceString;
        public uint StateFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string DeviceID;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string DeviceKey;
    }

    private const uint DISPLAY_DEVICE_ATTACHED_TO_DESKTOP = 0x1;
    private const uint DISPLAY_DEVICE_ACTIVE = 0x80000000;

    [StructLayout(LayoutKind.Sequential)]
    private struct RECT
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MONITORINFOEX
    {
        public int Size;
        public RECT Monitor;
        public RECT WorkArea;
        public uint Flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
        public string DeviceName;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern bool GetMonitorInfo(IntPtr hMonitor, ref MONITORINFOEX lpMonitorInfo);

    [DllImport("user32.dll")]
    private static extern int EnumDisplayMonitors(
        IntPtr hdc, IntPtr lprcClip, MonitorEnumProc lpfnEnum, IntPtr dwData);

    private delegate bool MonitorEnumProc(
        IntPtr hMonitor, IntPtr hdcMonitor, IntPtr lprcMonitor, IntPtr dwData);

    public static IReadOnlyList<string> GetAvailableMonitorDescriptions()
    {
        var descriptions = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        foreach (var monitorHandle in GetDisplayMonitorHandles())
        {
            var monitors = GetPhysicalMonitors(monitorHandle);
            if (monitors == null)
            {
                var error = Marshal.GetLastWin32Error();
                var deviceName = GetMonitorDeviceName(monitorHandle);
                Console.Error.WriteLine(
                    $"GetPhysicalMonitorsFromHMONITOR failed for {deviceName} (error {error})");

                continue;
            }

            try
            {
                foreach (var monitor in monitors)
                {
                    if (monitor.hPhysicalMonitor != IntPtr.Zero &&
                        !string.IsNullOrWhiteSpace(monitor.szPhysicalMonitorDescription))
                    {
                        descriptions.Add(monitor.szPhysicalMonitorDescription);
                    }
                }
            }
            finally
            {
                DestroyPhysicalMonitors((uint)monitors.Length, monitors);
            }
        }

        foreach (var desc in GetMonitorDescriptionsFromDisplayDevices())
        {
            descriptions.Add(desc);
        }

        return descriptions.ToList();
    }

    public async Task<bool> SetInputSourceAsync(byte inputSource)
    {
        return await Task.Run(() =>
            ForEachTargetMonitor(monitor =>
                SetVCPFeature(monitor.hPhysicalMonitor, MonitorInputSource.VcpCode, inputSource)));
    }

    public async Task<byte> GetInputSourceAsync()
    {
        byte result = 0;
        await Task.Run(() =>
            ForEachTargetMonitor(monitor =>
            {
                if (GetVCPFeatureAndVCPFeatureReply(
                    monitor.hPhysicalMonitor, MonitorInputSource.VcpCode,
                    out _, out var currentValue, out _))
                {
                    result = (byte)currentValue;
                    return true;
                }

                return false;
            }));
        return result;
    }

    private bool ForEachTargetMonitor(Func<PHYSICAL_MONITOR, bool> action)
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
                    if (monitor.hPhysicalMonitor == IntPtr.Zero)
                        continue;

                    if (!string.IsNullOrWhiteSpace(targetName) &&
                        monitor.szPhysicalMonitorDescription != null &&
                        !monitor.szPhysicalMonitorDescription.Contains(targetName, StringComparison.OrdinalIgnoreCase))
                    {
                        continue;
                    }

                    if (action(monitor))
                        return true;
                }
            }
            finally
            {
                DestroyPhysicalMonitors((uint)monitors.Length, monitors);
            }
        }

        return false;
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

    private static string GetMonitorDeviceName(IntPtr hMonitor)
    {
        var info = new MONITORINFOEX { Size = Marshal.SizeOf<MONITORINFOEX>() };
        if (GetMonitorInfo(hMonitor, ref info))
            return info.DeviceName;
        return $"hMonitor:{hMonitor}";
    }

    private static PHYSICAL_MONITOR[]? GetPhysicalMonitors(IntPtr hMonitor)
    {
        uint count = 0;
        if (!GetNumberOfPhysicalMonitorsFromHMONITOR(hMonitor, ref count) || count == 0)
            return null;

        var monitors = new PHYSICAL_MONITOR[count];
        if (!GetPhysicalMonitorsFromHMONITOR(hMonitor, count, monitors))
            return null;

        return monitors;
    }

    private static IEnumerable<string> GetMonitorDescriptionsFromDisplayDevices()
    {
        var descriptions = new List<string>();
        uint adapterNum = 0;

        var adapter = new DISPLAY_DEVICE { cb = Marshal.SizeOf<DISPLAY_DEVICE>() };

        while (EnumDisplayDevices(null, adapterNum, ref adapter, EDS_ATTACHEDTO_DESKTOP))
        {
            uint monitorNum = 0;
            var monitor = new DISPLAY_DEVICE { cb = Marshal.SizeOf<DISPLAY_DEVICE>() };

            while (EnumDisplayDevices(adapter.DeviceName, monitorNum, ref monitor, 0))
            {
                if ((monitor.StateFlags & DISPLAY_DEVICE_ACTIVE) != 0 &&
                    (monitor.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP) != 0 &&
                    !string.IsNullOrWhiteSpace(monitor.DeviceString))
                {
                    descriptions.Add(monitor.DeviceString);
                }

                monitor = new DISPLAY_DEVICE { cb = Marshal.SizeOf<DISPLAY_DEVICE>() };
                monitorNum++;
            }

            adapter = new DISPLAY_DEVICE { cb = Marshal.SizeOf<DISPLAY_DEVICE>() };
            adapterNum++;
        }

        return descriptions;
    }
}