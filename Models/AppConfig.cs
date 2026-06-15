using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using Serilog;

namespace KwmSwitcher.Models;

public class AppConfig
{
    public byte LocalInputSource { get; set; } = MonitorInputSource.DisplayPort;
    public byte RemoteInputSource { get; set; } = MonitorInputSource.Hdmi1;
    public InputSwitchProtocol InputProtocol { get; set; } = InputSwitchProtocol.Standard;
    public List<string> TrackedDeviceKeys { get; set; } = [];
    public int PollIntervalMs { get; set; } = 1000;
    public bool StartMinimized { get; set; } = true;
    public bool AutoStart { get; set; }
    public string? TargetMonitorName { get; set; }

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
}
