using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using KwmSwitcher.Models;

namespace KwmSwitcher.Services.Linux;

public class LinuxUsbMonitor : IUsbMonitor
{
    private Timer? _pollTimer;
    private HashSet<string> _lastDeviceKeys = [];
    private int _pollIntervalMs = 1000;

    public event Action<IEnumerable<UsbDeviceInfo>>? DevicesChanged;

    public void Start()
    {
        _lastDeviceKeys = [..GetCurrentDevices().Select(d => d.Key)];
        _pollTimer = new Timer(Poll, null, 0, _pollIntervalMs);
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
            catch
            {
            }
        }

        return devices;
    }

    private void Poll(object? state)
    {
        var current = GetCurrentDevices();
        var currentKeys = current.Select(d => d.Key).ToHashSet();

        if (!currentKeys.SetEquals(_lastDeviceKeys))
        {
            _lastDeviceKeys = currentKeys;
            DevicesChanged?.Invoke(current);
        }
    }

    public void Dispose()
    {
        Stop();
    }
}
