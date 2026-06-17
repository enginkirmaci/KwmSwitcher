using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.CompilerServices;
using System.Text.Json;
using Serilog;

namespace KwmSwitcher.Models;

/// <summary>
/// Persisted application settings (JSON, see <see cref="ConfigPath"/>).
///
/// Implements <see cref="INotifyPropertyChanged"/> so live consumers can react
/// to settings edited at runtime (e.g. a future live-poll-interval change).
/// The shared instance is the single source of truth: <see cref="SwitcherEngine"/>
/// already reads fields lazily at call-time, so input-source / protocol changes
/// propagate automatically.
/// </summary>
public sealed class AppConfig : INotifyPropertyChanged
{
    private byte _localInputSource = MonitorInputSource.DisplayPort;
    private byte _remoteInputSource = MonitorInputSource.Hdmi1;
    private InputSwitchProtocol _inputProtocol = InputSwitchProtocol.Standard;
    private List<string> _trackedDeviceKeys = [];
    private int _pollIntervalMs = 1000;
    private bool _startMinimized = true;
    private bool _autoStart;
    private string? _targetMonitorName;

    public byte LocalInputSource
    {
        get => _localInputSource;
        set => SetField(ref _localInputSource, value);
    }

    public byte RemoteInputSource
    {
        get => _remoteInputSource;
        set => SetField(ref _remoteInputSource, value);
    }

    public InputSwitchProtocol InputProtocol
    {
        get => _inputProtocol;
        set => SetField(ref _inputProtocol, value);
    }

    public List<string> TrackedDeviceKeys
    {
        get => _trackedDeviceKeys;
        set => SetField(ref _trackedDeviceKeys, value ?? []);
    }

    public int PollIntervalMs
    {
        get => _pollIntervalMs;
        set => SetField(ref _pollIntervalMs, value);
    }

    public bool StartMinimized
    {
        get => _startMinimized;
        set => SetField(ref _startMinimized, value);
    }

    // Cached hint only. The OS (registry / .desktop file) is the real source of
    // truth — read via IAutoStartService.IsEnabled().
    public bool AutoStart
    {
        get => _autoStart;
        set => SetField(ref _autoStart, value);
    }

    public string? TargetMonitorName
    {
        get => _targetMonitorName;
        set => SetField(ref _targetMonitorName, value);
    }

    private static readonly string ConfigDir = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "KwmSwitcher");

    private static readonly string ConfigPath = Path.Combine(ConfigDir, "config.json");

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true
    };

    public static AppConfig Load()
    {
        if (!File.Exists(ConfigPath))
            return new AppConfig();

        try
        {
            var json = File.ReadAllText(ConfigPath);
            var config = JsonSerializer.Deserialize<AppConfig>(json, JsonOptions) ?? new AppConfig();
            // Ensure TrackedDeviceKeys is never null (JSON deserialization can set it to null)
            config.TrackedDeviceKeys ??= [];
            return config;
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to load config from {Path}", ConfigPath);
            return new AppConfig();
        }
    }

    public void Save()
    {
        Directory.CreateDirectory(ConfigDir);
        var json = JsonSerializer.Serialize(this, JsonOptions);
        File.WriteAllText(ConfigPath, json);
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null)
        => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));

    private bool SetField<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
            return false;
        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }
}
