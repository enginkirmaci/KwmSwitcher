using System;
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
    private static extern bool SetVCPFeatureAndVCPFeatureReply(
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
            var monitorHandle = GetPrimaryMonitorHandle();
            if (monitorHandle == IntPtr.Zero)
                return false;

            var monitors = new PHYSICAL_MONITOR[1];
            try
            {
                if (!GetPhysicalMonitorsFromHMONITOR(monitorHandle, 1, monitors))
                    return false;

                return SetVCPFeatureAndVCPFeatureReply(
                    monitors[0].hPhysicalMonitor,
                    Models.MonitorInputSource.VcpCode,
                    inputSource);
            }
            finally
            {
                if (monitors[0].hPhysicalMonitor != IntPtr.Zero)
                    DestroyPhysicalMonitors(1, monitors);
            }
        });
    }

    public async Task<byte> GetInputSourceAsync()
    {
        return await Task.Run(() =>
        {
            var monitorHandle = GetPrimaryMonitorHandle();
            if (monitorHandle == IntPtr.Zero)
                return (byte)0;

            var monitors = new PHYSICAL_MONITOR[1];
            try
            {
                if (!GetPhysicalMonitorsFromHMONITOR(monitorHandle, 1, monitors))
                    return (byte)0;

                if (GetVCPFeatureAndVCPFeatureReply(
                    monitors[0].hPhysicalMonitor,
                    Models.MonitorInputSource.VcpCode,
                    out _, out var currentValue, out _))
                {
                    return (byte)currentValue;
                }

                return (byte)0;
            }
            finally
            {
                if (monitors[0].hPhysicalMonitor != IntPtr.Zero)
                    DestroyPhysicalMonitors(1, monitors);
            }
        });
    }

    private static IntPtr GetPrimaryMonitorHandle()
    {
        var monitorHandle = IntPtr.Zero;

        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (hMon, _, _, _) =>
        {
            monitorHandle = hMon;
            return false;
        }, IntPtr.Zero);

        return monitorHandle;
    }
}
