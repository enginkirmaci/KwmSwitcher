using System;
using System.Collections.Generic;
using System.Linq;
using KwmSwitcher.Models;
using Serilog;

namespace KwmSwitcher.Services;

/// <summary>
/// Shared backbone for platform USB monitors.
///
/// Concrete subclasses implement <see cref="GetCurrentDevices"/> (platform I/O)
/// and a triggering mechanism (<see cref="System.Threading.Timer"/> poll,
/// WMI events + debounce, …). When a change is detected they call
/// <see cref="RaiseIfChanged"/>, which owns the snapshot comparison + event
/// dispatch + error handling. This keeps the device-set-diff logic — previously
/// duplicated verbatim between the Linux poller and the Windows debounce
/// callback — in exactly one place.
/// </summary>
public abstract class UsbMonitorBase : IUsbMonitor
{
    private HashSet<string> _lastDeviceKeys = [];

    public event Action<IEnumerable<UsbDeviceInfo>>? DevicesChanged;

    /// <summary>
    /// Captures the current device set as the baseline so the first trigger
    /// only raises on a real change. Subclasses must call this after wiring up
    /// their trigger (and before returning from <c>Start</c>).
    /// </summary>
    protected void SeedBaseline()
    {
        _lastDeviceKeys = [..GetCurrentDevices().Select(d => d.Key)];
    }

    /// <summary>
    /// Compares the live device set against the last-seen snapshot and raises
    /// <see cref="DevicesChanged"/> when they differ. Any exception is logged
    /// and swallowed so a poll/event callback never tears down the monitor.
    /// </summary>
    protected void RaiseIfChanged()
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
            Log.Error(ex, "Error checking for USB device changes");
        }
    }

    public abstract IReadOnlyList<UsbDeviceInfo> GetCurrentDevices();
    public abstract void Start();
    public abstract void Stop();
    public abstract void Dispose();
}
