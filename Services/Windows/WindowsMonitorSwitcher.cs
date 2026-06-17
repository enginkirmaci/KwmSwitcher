using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.Text;
using System.Threading.Tasks;
using KwmSwitcher.Models;

namespace KwmSwitcher.Services.Windows;

/// <summary>
/// Switches monitor input source on Windows using SetupAPI + CreateFile + DeviceIoControl.
///
/// Why not dxva2.dll (GetPhysicalMonitorsFromHMONITOR / SetVCPFeature)?
/// On Windows 11, GetPhysicalMonitorsFromHMONITOR returns hPhysicalMonitor = IntPtr.Zero
/// for certain monitors (especially USB-C / DP connections), making SetVCPFeature impossible.
/// The DeviceIoControl approach opens the monitor device directly via CreateFile and sends
/// DDC/CI VCP commands through kernel IOCTLs — bypassing the broken dxva2.dll path entirely.
/// </summary>
[SupportedOSPlatform("windows")]
public class WindowsMonitorSwitcher : IMonitorSwitcher
{
    private readonly AppConfig _config;

    public WindowsMonitorSwitcher(AppConfig config)
    {
        _config = config;
    }

    #region SetupAPI P/Invoke

    private static readonly Guid GUID_DEVINTERFACE_MONITOR =
        new("E6F07B5F-EE97-4a90-B076-33F57BF4EAA7");

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr SetupDiGetClassDevs(
        ref Guid ClassGuid, IntPtr Enumerator, IntPtr hwndParent, uint Flags);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiEnumDeviceInterfaces(
        IntPtr DeviceInfoSet, IntPtr DeviceInfoData, ref Guid InterfaceClassGuid,
        uint MemberIndex, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData);

    // Overload 1: detail buffer = IntPtr, devInfoData = IntPtr (for querying required size)
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiGetDeviceInterfaceDetail(
        IntPtr DeviceInfoSet, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData,
        IntPtr DeviceInterfaceDetailData, uint DeviceInterfaceDetailDataSize,
        out uint RequiredSize, IntPtr DeviceInfoData);

    // Overload 2: detail buffer = IntPtr, devInfoData = ref (for retrieving actual data)
    [DllImport("setupapi.dll", EntryPoint = "SetupDiGetDeviceInterfaceDetailW",
        CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiGetDeviceInterfaceDetailWithInfo(
        IntPtr DeviceInfoSet, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData,
        IntPtr DeviceInterfaceDetailData, uint DeviceInterfaceDetailDataSize,
        out uint RequiredSize, ref SP_DEVINFO_DATA DeviceInfoData);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiGetDeviceRegistryProperty(
        IntPtr DeviceInfoSet, ref SP_DEVINFO_DATA DeviceInfoData,
        uint Property, out uint PropertyRegDataType,
        byte[] PropertyBuffer, uint PropertyBufferSize, out uint RequiredSize);

    [DllImport("setupapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr DeviceInfoSet);

    [StructLayout(LayoutKind.Sequential)]
    private struct SP_DEVICE_INTERFACE_DATA
    {
        public uint cbSize;
        public Guid InterfaceClassGuid;
        public uint Flags;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct SP_DEVINFO_DATA
    {
        public uint cbSize;
        public Guid ClassGuid;
        public uint DevInst;
        public IntPtr Reserved;
    }

    private const uint DIGCF_PRESENT = 0x02;
    private const uint DIGCF_DEVICEINTERFACE = 0x10;
    private const uint SPDRP_DEVICEDESC = 0x00;
    private const uint SPDRP_FRIENDLYNAME = 0x0C;

    #endregion

    #region Kernel32 P/Invoke

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateFile(
        string lpFileName, uint dwDesiredAccess, uint dwShareMode,
        IntPtr lpSecurityAttributes, uint dwCreationDisposition,
        uint dwFlagsAndAttributes, IntPtr hTemplateFile);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DeviceIoControl(
        IntPtr hDevice, uint dwIoControlCode,
        byte[] lpInBuffer, uint nInBufferSize,
        byte[] lpOutBuffer, uint nOutBufferSize,
        out uint lpBytesReturned, IntPtr lpOverlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr hObject);

    private const uint GENERIC_READ = 0x80000000;
    private const uint GENERIC_WRITE = 0x40000000;
    private const uint FILE_SHARE_READ = 0x01;
    private const uint FILE_SHARE_WRITE = 0x02;
    private const uint OPEN_EXISTING = 3;
    private static readonly IntPtr INVALID_HANDLE_VALUE = new(-1);

    #endregion

    #region IOCTL Definitions

    // CTL_CODE(FILE_DEVICE_VIDEO, function, METHOD_BUFFERED, FILE_ANY_ACCESS)
    private static uint CTL_CODE(uint deviceType, uint function, uint method, uint access)
        => (deviceType << 16) | (access << 14) | (function << 2) | method;

    private static readonly uint IOCTL_VIDEO_SET_VCP_FEATURE =
        CTL_CODE(0x23, 0x00A3, 0, 0);  // 0x0023028C

    private static readonly uint IOCTL_VIDEO_GET_VCP_FEATURE =
        CTL_CODE(0x23, 0x00A2, 0, 0);  // 0x00230288

    #endregion

    // Lightweight record to carry monitor metadata without holding native resources
    private record MonitorDevice(string DevicePath, string Description);

    /// <summary>
    /// Lists the descriptions of all currently connected monitors.
    /// Uses SetupAPI to enumerate monitor device interfaces — no dxva2.dll dependency.
    /// </summary>
    public static IReadOnlyList<string> GetAvailableMonitorDescriptions()
    {
        var descriptions = new List<string>();

        foreach (var device in EnumerateMonitorDevices())
        {
            if (!string.IsNullOrWhiteSpace(device.Description))
                descriptions.Add(device.Description);
        }

        return descriptions;
    }

    public async Task<bool> SetInputSourceAsync(byte inputSource)
    {
        return await Task.Run(() =>
        {
            var vcpCode = MonitorInputSource.GetVcpCode(_config.InputProtocol);
            var value = MonitorInputSource.GetProtocolValue(_config.InputProtocol, inputSource);

            foreach (var device in EnumerateMonitorDevices())
            {
                if (!MatchesTarget(device))
                    continue;

                var handle = OpenMonitorHandle(device.DevicePath);
                if (handle == INVALID_HANDLE_VALUE || handle == IntPtr.Zero)
                    continue;

                try
                {
                    if (SetVcpFeatureViaIoctl(handle, vcpCode, value))
                        return true;
                }
                finally
                {
                    CloseHandle(handle);
                }
            }

            return false;
        });
    }

    public async Task<byte> GetInputSourceAsync()
    {
        return await Task.Run(() =>
        {
            var vcpCode = MonitorInputSource.GetVcpCode(_config.InputProtocol);

            foreach (var device in EnumerateMonitorDevices())
            {
                if (!MatchesTarget(device))
                    continue;

                var handle = OpenMonitorHandle(device.DevicePath);
                if (handle == INVALID_HANDLE_VALUE || handle == IntPtr.Zero)
                    continue;

                try
                {
                    if (GetVcpFeatureViaIoctl(handle, vcpCode, out var currentValue))
                        return MonitorInputSource.DecodeInputSource(_config.InputProtocol, (byte)currentValue);
                }
                finally
                {
                    CloseHandle(handle);
                }
            }

            return (byte)0;
        });
    }

    public async Task<byte> GetPipModeAsync()
    {
        return await Task.Run(() =>
        {
            var vcpCode = MonitorInputSource.GetPipVcpCode(_config.InputProtocol);

            foreach (var device in EnumerateMonitorDevices())
            {
                if (!MatchesTarget(device))
                    continue;

                var handle = OpenMonitorHandle(device.DevicePath);
                if (handle == INVALID_HANDLE_VALUE || handle == IntPtr.Zero)
                    continue;

                try
                {
                    if (GetVcpFeatureViaIoctl(handle, vcpCode, out var currentValue))
                        return MonitorInputSource.DecodePipMode(_config.InputProtocol, (byte)currentValue);
                }
                finally
                {
                    CloseHandle(handle);
                }
            }

            return (byte)0;
        });
    }

    public async Task<bool> SetPipModeAsync(byte mode)
    {
        return await Task.Run(() =>
        {
            var vcpCode = MonitorInputSource.GetPipVcpCode(_config.InputProtocol);
            var value = MonitorInputSource.GetPipProtocolValue(_config.InputProtocol, mode);

            foreach (var device in EnumerateMonitorDevices())
            {
                if (!MatchesTarget(device))
                    continue;

                var handle = OpenMonitorHandle(device.DevicePath);
                if (handle == INVALID_HANDLE_VALUE || handle == IntPtr.Zero)
                    continue;

                try
                {
                    if (SetVcpFeatureViaIoctl(handle, vcpCode, value))
                        return true;
                }
                finally
                {
                    CloseHandle(handle);
                }
            }

            return false;
        });
    }

    private bool MatchesTarget(MonitorDevice device)
    {
        var targetName = _config.TargetMonitorName;
        if (string.IsNullOrWhiteSpace(targetName))
            return true;

        return !string.IsNullOrEmpty(device.Description) &&
               device.Description.Contains(targetName, StringComparison.OrdinalIgnoreCase);
    }

    #region Monitor enumeration via SetupAPI

    /// <summary>
    /// Enumerates all present monitor devices using SetupDiGetClassDevs +
    /// SetupDiEnumDeviceInterfaces.  Returns device path + friendly description
    /// without holding any native handles — safe for yield return.
    /// </summary>
    private static IEnumerable<MonitorDevice> EnumerateMonitorDevices()
    {
        var interfaceClassGuid = GUID_DEVINTERFACE_MONITOR;

        var hDevInfo = SetupDiGetClassDevs(
            ref interfaceClassGuid,
            IntPtr.Zero,
            IntPtr.Zero,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);

        if (hDevInfo == INVALID_HANDLE_VALUE)
            yield break;

        try
        {
            uint index = 0;
            while (true)
            {
                var ifaceData = new SP_DEVICE_INTERFACE_DATA
                {
                    cbSize = (uint)Marshal.SizeOf<SP_DEVICE_INTERFACE_DATA>()
                };

                if (!SetupDiEnumDeviceInterfaces(
                        hDevInfo, IntPtr.Zero,
                        ref interfaceClassGuid, index, ref ifaceData))
                {
                    break;  // No more interfaces
                }
                index++;

                // --- Step 1: query required buffer size ---
                SetupDiGetDeviceInterfaceDetail(
                    hDevInfo, ref ifaceData,
                    IntPtr.Zero, 0, out var requiredSize, IntPtr.Zero);

                if (requiredSize == 0)
                    continue;

                // --- Step 2: allocate and populate detail data ---
                var detailBuffer = Marshal.AllocHGlobal((int)requiredSize);
                try
                {
                    // cbSize for SP_DEVICE_INTERFACE_DETAIL_DATA:
                    //   x64 → 8, x86 → 6  (per MSDN)
                    Marshal.WriteInt32(detailBuffer, IntPtr.Size == 8 ? 8 : 6);

                    var devInfoData = new SP_DEVINFO_DATA
                    {
                        cbSize = (uint)Marshal.SizeOf<SP_DEVINFO_DATA>()
                    };

                    if (!SetupDiGetDeviceInterfaceDetailWithInfo(
                            hDevInfo, ref ifaceData,
                            detailBuffer, requiredSize, out _, ref devInfoData))
                    {
                        continue;
                    }

                    // DevicePath starts right after the DWORD cbSize at offset 4
                    var devicePath = Marshal.PtrToStringUni(detailBuffer + 4);
                    if (string.IsNullOrEmpty(devicePath))
                        continue;

                    var description = GetDeviceFriendlyName(hDevInfo, ref devInfoData)
                                      ?? devicePath;

                    yield return new MonitorDevice(devicePath, description);
                }
                finally
                {
                    Marshal.FreeHGlobal(detailBuffer);
                }
            }
        }
        finally
        {
            SetupDiDestroyDeviceInfoList(hDevInfo);
        }
    }

    private static string? GetDeviceFriendlyName(IntPtr hDevInfo, ref SP_DEVINFO_DATA devInfoData)
    {
        return GetDeviceProperty(hDevInfo, ref devInfoData, SPDRP_FRIENDLYNAME)
            ?? GetDeviceProperty(hDevInfo, ref devInfoData, SPDRP_DEVICEDESC);
    }

    private static string? GetDeviceProperty(
        IntPtr hDevInfo, ref SP_DEVINFO_DATA devInfoData, uint property)
    {
        SetupDiGetDeviceRegistryProperty(
            hDevInfo, ref devInfoData, property,
            out _, null, 0, out var requiredSize);

        if (requiredSize == 0)
            return null;

        var buffer = new byte[requiredSize];
        if (!SetupDiGetDeviceRegistryProperty(
                hDevInfo, ref devInfoData, property,
                out _, buffer, requiredSize, out _))
        {
            return null;
        }

        return Encoding.Unicode.GetString(buffer).TrimEnd('\0');
    }

    #endregion

    #region VCP via DeviceIoControl

    private static IntPtr OpenMonitorHandle(string devicePath)
    {
        // \\?\ prefix from SetupAPI works with CreateFile for monitor device paths.
        // If access denied, the process may need to run elevated (administrator).
        return CreateFile(
            devicePath,
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            IntPtr.Zero,
            OPEN_EXISTING,
            0,
            IntPtr.Zero);
    }

    /// <summary>
    /// Sends IOCTL_VIDEO_SET_VCP_FEATURE.
    /// Input buffer: VIDEO_VCP_FEATURE { ULONG VCPCode; ULONG FeatureValue; }
    /// </summary>
    private static bool SetVcpFeatureViaIoctl(IntPtr hMonitor, byte vcpCode, uint value)
    {
        var inBuffer = new byte[8];
        BitConverter.GetBytes((uint)vcpCode).CopyTo(inBuffer, 0);
        BitConverter.GetBytes(value).CopyTo(inBuffer, 4);

        return DeviceIoControl(
            hMonitor, IOCTL_VIDEO_SET_VCP_FEATURE,
            inBuffer, (uint)inBuffer.Length,
            null, 0, out _, IntPtr.Zero);
    }

    /// <summary>
    /// Sends IOCTL_VIDEO_GET_VCP_FEATURE.
    /// Input:  ULONG VCPCode
    /// Output: VIDEO_VCP_FEATURE_VALUE { ULONG VCPCode; ULONG CurrentValue; ULONG MaximumValue; ULONG VCPType; }
    /// </summary>
    private static bool GetVcpFeatureViaIoctl(IntPtr hMonitor, byte vcpCode, out uint currentValue)
    {
        currentValue = 0;

        var inBuffer = new byte[4];
        BitConverter.GetBytes((uint)vcpCode).CopyTo(inBuffer, 0);

        var outBuffer = new byte[16];
        if (!DeviceIoControl(
                hMonitor, IOCTL_VIDEO_GET_VCP_FEATURE,
                inBuffer, (uint)inBuffer.Length,
                outBuffer, (uint)outBuffer.Length,
                out _, IntPtr.Zero))
        {
            return false;
        }

        currentValue = BitConverter.ToUInt32(outBuffer, 4);  // CurrentValue at offset 4
        return true;
    }

    #endregion
}
