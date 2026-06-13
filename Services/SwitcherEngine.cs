using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using KwmSwitcher.Models;

namespace KwmSwitcher.Services;

public class SwitcherEngine : IDisposable
{
    private readonly IUsbMonitor _usbMonitor;
    private readonly IMonitorSwitcher _monitorSwitcher;
    private readonly AppConfig _config;
    private bool _localActive;

    public event Action<bool>? LocalActiveChanged;
    public event Action<string>? StatusChanged;

    public bool IsLocalActive => _localActive;

    public SwitcherEngine(IUsbMonitor usbMonitor, IMonitorSwitcher monitorSwitcher, AppConfig config)
    {
        _usbMonitor = usbMonitor;
        _monitorSwitcher = monitorSwitcher;
        _config = config;
    }

    public void Start()
    {
        _usbMonitor.DevicesChanged += OnDevicesChanged;
        _usbMonitor.Start();

        var devices = _usbMonitor.GetCurrentDevices();
        EvaluateState(devices);
    }

    public void Stop()
    {
        _usbMonitor.DevicesChanged -= OnDevicesChanged;
        _usbMonitor.Stop();
    }

    public async Task SwitchToLocalAsync()
    {
        StatusChanged?.Invoke($"Switching monitor to {MonitorInputSource.GetName(_config.LocalInputSource)}...");
        var success = await _monitorSwitcher.SetInputSourceAsync(_config.LocalInputSource);
        if (success)
        {
            _localActive = true;
            LocalActiveChanged?.Invoke(true);
            StatusChanged?.Invoke($"Monitor set to {MonitorInputSource.GetName(_config.LocalInputSource)} (local)");
        }
        else
        {
            StatusChanged?.Invoke("Failed to switch monitor input");
        }
    }

    public async Task SwitchToRemoteAsync()
    {
        StatusChanged?.Invoke($"Switching monitor to {MonitorInputSource.GetName(_config.RemoteInputSource)}...");
        var success = await _monitorSwitcher.SetInputSourceAsync(_config.RemoteInputSource);
        if (success)
        {
            _localActive = false;
            LocalActiveChanged?.Invoke(false);
            StatusChanged?.Invoke($"Monitor set to {MonitorInputSource.GetName(_config.RemoteInputSource)} (remote)");
        }
        else
        {
            StatusChanged?.Invoke("Failed to switch monitor input");
        }
    }

    private void OnDevicesChanged(IEnumerable<UsbDeviceInfo> devices)
    {
        EvaluateState(devices);
    }

    private void EvaluateState(IEnumerable<UsbDeviceInfo> devices)
    {
        var trackedKeys = _config.TrackedDeviceKeys.ToHashSet(StringComparer.OrdinalIgnoreCase);

        if (trackedKeys.Count == 0)
        {
            StatusChanged?.Invoke("No tracked devices configured. Open settings to select USB devices.");
            return;
        }

        var currentKeys = devices.Select(d => d.Key).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var anyTrackedPresent = trackedKeys.Any(k => currentKeys.Contains(k));

        if (anyTrackedPresent && !_localActive)
        {
            _ = SwitchToLocalAsync();
        }
        else if (!anyTrackedPresent && _localActive)
        {
            _ = SwitchToRemoteAsync();
        }
        else
        {
            StatusChanged?.Invoke(anyTrackedPresent
                ? "Local machine active, tracked USB devices detected"
                : "Remote machine active, no tracked USB devices");
        }
    }

    public void Dispose()
    {
        Stop();
    }
}
