using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using KwmSwitcher.Models;
using Serilog;

namespace KwmSwitcher.Services.Linux;

public class LinuxUsbMonitor : IUsbMonitor
{
    private readonly AppConfig _config;
    private Timer? _pollTimer;
    private HashSet<string> _lastDeviceKeys = [];

    public LinuxUsbMonitor(AppConfig config)
    {
        _config = config;
    }

    public event Action<IEnumerable<UsbDeviceInfo>>? DevicesChanged;

    public void Start()
    {
        _lastDeviceKeys = [..GetCurrentDevices().Select(d => d.Key)];
        // Poll interval is read at start; a restart picks up any edited value.
        _pollTimer = new Timer(Poll, null, 0, _config.PollIntervalMs);
    }

    public void Stop()
    {
        _pollTimer?.Dispose();
        _pollTimer = null;
    }

    public IReadOnlyList<UsbDeviceInfo> GetCurrentDevices()
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

    private void Poll(object? state)
    {
        try
        {
            var current = GetCurrentDevices();
            var currentKeys = current.Select(d => d.Key).ToHashSet();

            if (!currentKeys.SetEquals(_lastDeviceKeys))
            {
                _lastDeviceKeys = currentKeys;
                DevicesChanged?.Invoke(current);
            }
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Error in USB poll callback");
        }
    }

    public void Dispose()
    {
        Stop();
    }
}
