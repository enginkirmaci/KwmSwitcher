using System;
using System.Collections.Generic;
using System.Management;
using System.Linq;
using System.Runtime.Versioning;
using System.Threading;
using KwmSwitcher.Models;

namespace KwmSwitcher.Services.Windows;

[SupportedOSPlatform("windows")]
public class WindowsUsbMonitor : IUsbMonitor
{
    private ManagementEventWatcher? _insertWatcher;
    private ManagementEventWatcher? _removeWatcher;
    private Timer? _debounceTimer;
    private HashSet<string> _lastDeviceKeys = [];

    public event Action<IEnumerable<UsbDeviceInfo>>? DevicesChanged;

    public void Start()
    {
        _lastDeviceKeys = [..GetCurrentDevices().Select(d => d.Key)];

        var insertQuery = new WqlEventQuery(
            "SELECT * FROM __InstanceCreationEvent WITHIN 2 WHERE TargetInstance ISA 'Win32_PnPEntity'");
        _insertWatcher = new ManagementEventWatcher(insertQuery);
        _insertWatcher.EventArrived += OnDeviceEvent;
        _insertWatcher.Start();

        var removeQuery = new WqlEventQuery(
            "SELECT * FROM __InstanceDeletionEvent WITHIN 2 WHERE TargetInstance ISA 'Win32_PnPEntity'");
        _removeWatcher = new ManagementEventWatcher(removeQuery);
        _removeWatcher.EventArrived += OnDeviceEvent;
        _removeWatcher.Start();
    }

    public void Stop()
    {
        _insertWatcher?.Stop();
        _insertWatcher?.Dispose();
        _insertWatcher = null;

        _removeWatcher?.Stop();
        _removeWatcher?.Dispose();
        _removeWatcher = null;

        _debounceTimer?.Dispose();
        _debounceTimer = null;
    }

    public IReadOnlyList<UsbDeviceInfo> GetCurrentDevices()
    {
        var devices = new List<UsbDeviceInfo>();

        try
        {
            using var searcher = new ManagementObjectSearcher(
                "SELECT * FROM Win32_PnPEntity WHERE PNPDeviceID LIKE 'USB\\\\%'");

            foreach (var obj in searcher.Get())
            {
                var deviceId = obj["PNPDeviceID"]?.ToString() ?? "";
                var description = obj["Description"]?.ToString() ?? "USB Device";

                var parts = deviceId.Split('\\');
                if (parts.Length < 2)
                    continue;

                var idPart = parts[1];
                var vid = "";
                var pid = "";

                foreach (var segment in idPart.Split('&'))
                {
                    if (segment.StartsWith("VID_", StringComparison.OrdinalIgnoreCase))
                        vid = segment[4..].ToLowerInvariant();
                    else if (segment.StartsWith("PID_", StringComparison.OrdinalIgnoreCase))
                        pid = segment[4..].ToLowerInvariant();
                }

                if (!string.IsNullOrEmpty(vid) && !string.IsNullOrEmpty(pid))
                    devices.Add(new UsbDeviceInfo(vid, pid, description));
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Failed to enumerate USB devices: {ex.Message}");
        }

        return devices;
    }

    private void OnDeviceEvent(object sender, EventArrivedEventArgs e)
    {
        _debounceTimer?.Dispose();
        _debounceTimer = new Timer(_ =>
        {
            var current = GetCurrentDevices();
            var currentKeys = current.Select(d => d.Key).ToHashSet();

            if (!currentKeys.SetEquals(_lastDeviceKeys))
            {
                _lastDeviceKeys = currentKeys;
                DevicesChanged?.Invoke(current);
            }
        }, null, 500, Timeout.Infinite);
    }

    public void Dispose()
    {
        Stop();
    }
}
