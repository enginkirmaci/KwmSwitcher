using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using KwmSwitcher.Models;
using Serilog;

namespace KwmSwitcher.Services;

public class SwitcherEngine : IDisposable
{
    private readonly IUsbMonitor _usbMonitor;
    private readonly IMonitorSwitcher _monitorSwitcher;
    private readonly AppConfig _config;
    /// <summary>
    /// Marshals event notifications to the UI thread. Defaults to inline
    /// (synchronous) so the engine is testable without a UI sync context.
    /// </summary>
    private readonly Action<Action> _postToConsumer;
    private readonly SemaphoreSlim _switchLock = new(1, 1);
    private readonly SemaphoreSlim _pipLock = new(1, 1);
    private volatile bool _localActive;
    private volatile bool _stopped;
    private volatile byte _pipMode = MonitorInputSource.PipOff;
    private volatile bool _pipQueryFailed;
    private byte? _lastSetInputSource;
    private DateTime _lastSwitchTime = DateTime.MinValue;
    private readonly TimeSpan _switchCooldown = TimeSpan.FromSeconds(3);

    public event Action<bool>? LocalActiveChanged;
    public event Action<string>? StatusChanged;
    public event Action<byte>? PipModeChanged;

    public bool IsLocalActive => _localActive;
    public byte PipMode => _pipMode;
    public bool IsPipActive => MonitorInputSource.IsPipActive(_pipMode);
    public bool PipQueryFailed => _pipQueryFailed;

    public SwitcherEngine(IUsbMonitor usbMonitor, IMonitorSwitcher monitorSwitcher, AppConfig config,
                          Action<Action>? postToConsumer = null)
    {
        _usbMonitor = usbMonitor;
        _monitorSwitcher = monitorSwitcher;
        _config = config;
        _postToConsumer = postToConsumer ?? (action => action());
    }

    public void Start()
    {
        _stopped = false;
        _usbMonitor.DevicesChanged += OnDevicesChanged;
        _usbMonitor.Start();

        var devices = _usbMonitor.GetCurrentDevices();
        InitState(devices);

        _ = RefreshPipStateAsync();
    }

    public void Stop()
    {
        _stopped = true;
        _usbMonitor.DevicesChanged -= OnDevicesChanged;
        _usbMonitor.Stop();
    }

    public async Task SwitchToLocalAsync()
    {
        if (_stopped) return;
        if (!_switchLock.Wait(0)) return;
        try
        {
            if (_lastSetInputSource == _config.LocalInputSource)
            {
                _localActive = true;
                NotifyLocalActiveChanged(true);
                NotifyStatusChanged($"Monitor already on {MonitorInputSource.GetName(_config.LocalInputSource)} (local)");
                return;
            }

            NotifyStatusChanged($"Switching monitor to {MonitorInputSource.GetName(_config.LocalInputSource)}...");
            Log.Information("Switching monitor to {Input} (local)", MonitorInputSource.GetName(_config.LocalInputSource));

            var success = await _monitorSwitcher.SetInputSourceAsync(_config.LocalInputSource);

            if (_stopped) return;

            if (success)
            {
                _lastSetInputSource = _config.LocalInputSource;
                _localActive = true;
                _lastSwitchTime = DateTime.Now;
                NotifyLocalActiveChanged(true);
                NotifyStatusChanged($"Monitor set to {MonitorInputSource.GetName(_config.LocalInputSource)} (local)");
                Log.Information("Monitor switched to {Input} (local)", MonitorInputSource.GetName(_config.LocalInputSource));
            }
            else
            {
                NotifyStatusChanged("Failed to switch monitor input");
                Log.Warning("Failed to switch monitor to {Input} (local)", MonitorInputSource.GetName(_config.LocalInputSource));
            }
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Error switching to local");
            NotifyStatusChanged($"Error switching to local: {ex.Message}");
        }
        finally
        {
            try { _switchLock.Release(); }
            catch (ObjectDisposedException) { }
            catch (InvalidOperationException) { }
        }
    }

    public async Task SwitchToRemoteAsync()
    {
        if (_stopped) return;
        if (!_switchLock.Wait(0)) return;
        try
        {
            if (_lastSetInputSource == _config.RemoteInputSource)
            {
                _localActive = false;
                NotifyLocalActiveChanged(false);
                NotifyStatusChanged($"Monitor already on {MonitorInputSource.GetName(_config.RemoteInputSource)} (remote)");
                return;
            }

            NotifyStatusChanged($"Switching monitor to {MonitorInputSource.GetName(_config.RemoteInputSource)}...");
            Log.Information("Switching monitor to {Input} (remote)", MonitorInputSource.GetName(_config.RemoteInputSource));

            var success = await _monitorSwitcher.SetInputSourceAsync(_config.RemoteInputSource);

            if (_stopped) return;

            if (success)
            {
                _lastSetInputSource = _config.RemoteInputSource;
                _localActive = false;
                _lastSwitchTime = DateTime.Now;
                NotifyLocalActiveChanged(false);
                NotifyStatusChanged($"Monitor set to {MonitorInputSource.GetName(_config.RemoteInputSource)} (remote)");
                Log.Information("Monitor switched to {Input} (remote)", MonitorInputSource.GetName(_config.RemoteInputSource));
            }
            else
            {
                NotifyStatusChanged("Failed to switch monitor input");
                Log.Warning("Failed to switch monitor to {Input} (remote)", MonitorInputSource.GetName(_config.RemoteInputSource));
            }
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Error switching to remote");
            NotifyStatusChanged($"Error switching to remote: {ex.Message}");
        }
        finally
        {
            try { _switchLock.Release(); }
            catch (ObjectDisposedException) { }
            catch (InvalidOperationException) { }
        }
    }

    private void OnDevicesChanged(IEnumerable<UsbDeviceInfo> devices)
    {
        try
        {
            _ = RefreshPipStateAsync();
            EvaluateState(devices);
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Error in OnDevicesChanged");
        }
    }

    private void InitState(IEnumerable<UsbDeviceInfo> devices)
    {
        var trackedKeys = _config.TrackedDeviceKeys.ToHashSet(StringComparer.OrdinalIgnoreCase);

        if (trackedKeys.Count == 0)
        {
            NotifyStatusChanged("No tracked devices configured. Open settings to select USB devices.");
            return;
        }

        var currentKeys = devices.Select(d => d.Key).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var anyTrackedPresent = trackedKeys.Any(k => currentKeys.Contains(k));

        if (anyTrackedPresent)
        {
            _localActive = true;
            _lastSetInputSource = _config.LocalInputSource;
            NotifyStatusChanged("Local machine active, tracked USB devices detected");
        }
        else
        {
            _localActive = false;
            _lastSetInputSource = _config.RemoteInputSource;
            NotifyStatusChanged("Remote machine active, no tracked USB devices");
        }

        NotifyLocalActiveChanged(_localActive);
    }

    private void EvaluateState(IEnumerable<UsbDeviceInfo> devices)
    {
        if (_stopped) return;

        var trackedKeys = _config.TrackedDeviceKeys.ToHashSet(StringComparer.OrdinalIgnoreCase);

        if (trackedKeys.Count == 0)
        {
            NotifyStatusChanged("No tracked devices configured. Open settings to select USB devices.");
            return;
        }

        if (IsPipActive)
        {
            Log.Debug("PiP/PBP active ({Mode}), skipping automatic input switch",
                MonitorInputSource.GetPipModeName(_pipMode));
            NotifyStatusChanged($"PiP/PBP active ({MonitorInputSource.GetPipModeName(_pipMode)}), auto-switch suspended");
            return;
        }

        var currentKeys = devices.Select(d => d.Key).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var anyTrackedPresent = trackedKeys.Any(k => currentKeys.Contains(k));

        if (anyTrackedPresent && !_localActive)
        {
            if (DateTime.Now - _lastSwitchTime < _switchCooldown)
            {
                Log.Debug("Ignoring switch-to-local request, within cooldown period");
                return;
            }
            _ = SafeSwitchToLocalAsync();
        }
        else if (!anyTrackedPresent && _localActive)
        {
            if (DateTime.Now - _lastSwitchTime < _switchCooldown)
            {
                Log.Debug("Ignoring switch-to-remote request, within cooldown period");
                return;
            }
            _ = SafeSwitchToRemoteAsync();
        }
        else
        {
            NotifyStatusChanged(anyTrackedPresent
                ? "Local machine active, tracked USB devices detected"
                : "Remote machine active, no tracked USB devices");
        }
    }

    private void NotifyLocalActiveChanged(bool active)
    {
        try { _postToConsumer(() => LocalActiveChanged?.Invoke(active)); }
        catch (Exception ex) { Log.Error(ex, "Error invoking LocalActiveChanged"); }
    }

    private void NotifyStatusChanged(string status)
    {
        try { _postToConsumer(() => StatusChanged?.Invoke(status)); }
        catch (Exception ex) { Log.Error(ex, "Error invoking StatusChanged"); }
    }

    public void Dispose()
    {
        _stopped = true;
        Stop();
        _switchLock.Dispose();
        _pipLock.Dispose();
    }

    public async Task RefreshPipStateAsync()
    {
        if (_stopped) return;
        if (!_pipLock.Wait(0)) return;
        try
        {
            var mode = await _monitorSwitcher.GetPipModeAsync();
            if (_stopped) return;

            _pipQueryFailed = false;
            if (mode != _pipMode)
            {
                _pipMode = mode;
                Log.Information("PiP mode detected: {Mode}", MonitorInputSource.GetPipModeName(mode));
                NotifyPipModeChanged(mode);
            }
        }
        catch (Exception ex)
        {
            _pipQueryFailed = true;
            Log.Debug(ex, "PiP mode query not supported on this display");
        }
        finally
        {
            try { _pipLock.Release(); }
            catch (ObjectDisposedException) { }
            catch (InvalidOperationException) { }
        }
    }

    public async Task<bool> ActivatePipAsync()
    {
        return await SetPipModeAsync(MonitorInputSource.PipOn);
    }

    public async Task<bool> DeactivatePipAsync()
    {
        return await SetPipModeAsync(MonitorInputSource.PipOff);
    }

    public async Task<bool> TogglePipAsync()
    {
        if (IsPipActive)
            return await DeactivatePipAsync();
        return await ActivatePipAsync();
    }

    private async Task<bool> SetPipModeAsync(byte mode)
    {
        if (_stopped) return false;
        if (!_pipLock.Wait(0)) return false;
        try
        {
            NotifyStatusChanged($"Setting PiP mode to {MonitorInputSource.GetPipModeName(mode)}...");
            Log.Information("Setting PiP mode to {Mode}", MonitorInputSource.GetPipModeName(mode));

            var success = await _monitorSwitcher.SetPipModeAsync(mode);
            if (_stopped) return false;

            if (success)
            {
                _pipMode = MonitorInputSource.CanonicalizePipMode(_config.InputProtocol, mode);
                _pipQueryFailed = false;
                NotifyPipModeChanged(_pipMode);
                NotifyStatusChanged(IsPipActive
                    ? $"PiP/PBP active ({MonitorInputSource.GetPipModeName(_pipMode)}), auto-switch suspended"
                    : "PiP/PBP off, auto-switch resumed");
                Log.Information("PiP mode set to {Mode}", MonitorInputSource.GetPipModeName(_pipMode));
            }
            else
            {
                _pipQueryFailed = true;
                NotifyStatusChanged("Failed to set PiP mode (display may not support PiP over DDC/CI)");
                Log.Warning("Failed to set PiP mode to {Mode}", MonitorInputSource.GetPipModeName(mode));
            }
            return success;
        }
        catch (Exception ex)
        {
            _pipQueryFailed = true;
            Log.Error(ex, "Error setting PiP mode");
            NotifyStatusChanged($"Error setting PiP mode: {ex.Message}");
            return false;
        }
        finally
        {
            try { _pipLock.Release(); }
            catch (ObjectDisposedException) { }
            catch (InvalidOperationException) { }
        }
    }

    private void NotifyPipModeChanged(byte mode)
    {
        try { _postToConsumer(() => PipModeChanged?.Invoke(mode)); }
        catch (Exception ex) { Log.Error(ex, "Error invoking PipModeChanged"); }
    }

    private async Task SafeSwitchToLocalAsync()
    {
        try
        {
            await SwitchToLocalAsync();
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Unhandled error in SafeSwitchToLocalAsync");
        }
    }

    private async Task SafeSwitchToRemoteAsync()
    {
        try
        {
            await SwitchToRemoteAsync();
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Unhandled error in SafeSwitchToRemoteAsync");
        }
    }
}