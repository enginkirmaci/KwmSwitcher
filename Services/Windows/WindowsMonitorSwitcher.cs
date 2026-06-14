using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.Threading.Tasks;

namespace KwmSwitcher.Services.Windows;

[SupportedOSPlatform("windows")]
public class WindowsMonitorSwitcher : IMonitorSwitcher
{
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

    public async Task<bool> SetInputSourceAsync(byte inputSource)
    {
        return await Task.Run(() =>
        {
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

                        if (SetVCPFeature(
                            monitor.hPhysicalMonitor,
                            Models.MonitorInputSource.VcpCode,
                            inputSource))
                        {
                            return true;
                        }
                    }
                }
                finally
                {
                    DestroyPhysicalMonitors((uint)monitors.Length, monitors);
                }
            }

            return false;
        });
    }

    public async Task<byte> GetInputSourceAsync()
    {
        return await Task.Run(() =>
        {
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

                        if (GetVCPFeatureAndVCPFeatureReply(
                            monitor.hPhysicalMonitor,
                            Models.MonitorInputSource.VcpCode,
                            out _, out var currentValue, out _))
                        {
                            return (byte)currentValue;
                        }
                    }
                }
                finally
                {
                    DestroyPhysicalMonitors((uint)monitors.Length, monitors);
                }
            }

            return (byte)0;
        });
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
        if (!GetNumberOfPhysicalMonitorsFromHMONITOR(hMonitor, ref count) || count == 0)
            return null;

        var monitors = new PHYSICAL_MONITOR[count];
        if (!GetPhysicalMonitorsFromHMONITOR(hMonitor, count, monitors))
            return null;

        return monitors;
    }
}
