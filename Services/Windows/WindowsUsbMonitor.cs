using System;
using System.Collections.Generic;
using System.Linq;
using System.Management;
using System.Runtime.Versioning;
using System.Threading;
using KwmSwitcher.Models;
using Serilog;

namespace KwmSwitcher.Services.Windows;

/// <summary>
/// Reacts to WMI device insert/remove events. Each event arm/restarts a short
/// debounce timer; the debounce callback delegates to
/// <see cref="UsbMonitorBase.RaiseIfChanged"/> for diff bookkeeping.
/// </summary>
[SupportedOSPlatform("windows")]
public class WindowsUsbMonitor : UsbMonitorBase
{
    private ManagementEventWatcher? _insertWatcher;
    private ManagementEventWatcher? _removeWatcher;
    private Timer? _debounceTimer;

    public override void Start()
    {
        SeedBaseline();

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

    public override void Stop()
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

    public override IReadOnlyList<UsbDeviceInfo> GetCurrentDevices()
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
            Log.Error(ex, "Failed to enumerate USB devices");
            Console.Error.WriteLine($"Failed to enumerate USB devices: {ex.Message}");
        }

        return devices;
    }

    private void OnDeviceEvent(object sender, EventArrivedEventArgs e)
    {
        // Debounce: coalesce a burst of insert/remove events into one check.
        _debounceTimer?.Dispose();
        _debounceTimer = new Timer(_ => RaiseIfChanged(), null, 500, Timeout.Infinite);
    }

    public override void Dispose() => Stop();
}
