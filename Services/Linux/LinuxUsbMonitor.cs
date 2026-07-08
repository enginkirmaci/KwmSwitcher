using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using KwmSwitcher.Models;
using Serilog;

namespace KwmSwitcher.Services.Linux;

/// <summary>
/// Polls <c>/sys/bus/usb/devices</c> at a fixed interval. Diff bookkeeping and
/// event dispatch live in <see cref="UsbMonitorBase"/>; this class only knows
/// how to enumerate sysfs devices and when to trigger a check.
/// </summary>
public class LinuxUsbMonitor : UsbMonitorBase
{
    private readonly AppConfig _config;
    private Timer? _pollTimer;

    public LinuxUsbMonitor(AppConfig config)
    {
        _config = config;
    }

    public override void Start()
    {
        SeedBaseline();
        // Poll interval is read at start; a restart picks up any edited value.
        _pollTimer = new Timer(_ => RaiseIfChanged(), null, 0, _config.PollIntervalMs);
    }

    public override void Stop()
    {
        _pollTimer?.Dispose();
        _pollTimer = null;
    }

    public override IReadOnlyList<UsbDeviceInfo> GetCurrentDevices()
    {
        var devices = new List<UsbDeviceInfo>();
        var usbBase = "/sys/bus/usb/devices";

        if (!Directory.Exists(usbBase))
            return devices;

        foreach (var dir in Directory.GetDirectories(usbBase))
        {
            var idVendorPath = Path.Combine(dir, "idVendor");
            var idProductPath = Path.Combine(dir, "idProduct");
            var productPath = Path.Combine(dir, "product");

            if (!File.Exists(idVendorPath) || !File.Exists(idProductPath))
                continue;

            try
            {
                var vendorId = File.ReadAllText(idVendorPath).Trim();
                var productId = File.ReadAllText(idProductPath).Trim();
                var description = File.Exists(productPath)
                    ? File.ReadAllText(productPath).Trim()
                    : $"USB Device {vendorId}:{productId}";

                devices.Add(new UsbDeviceInfo(vendorId, productId, description));
            }
            catch (Exception ex)
            {
                Log.Warning(ex, "Failed to read USB device info from {Path}", dir);
            }
        }

        return devices;
    }

    public override void Dispose() => Stop();
}
