using System;
using System.Runtime.Versioning;
using Microsoft.Win32;
using Serilog;

namespace KwmSwitcher.Services.Windows;

[SupportedOSPlatform("windows")]
public class WindowsAutoStartService : IAutoStartService
{
    private const string RegistryKeyPath = @"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    private const string AppName = "KwmSwitcher";

    public bool IsEnabled()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(RegistryKeyPath, false);
            return key?.GetValue(AppName) != null;
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to check autostart status");
            return false;
        }
    }

    public void Enable()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(RegistryKeyPath, true)
                         ?? Registry.CurrentUser.CreateSubKey(RegistryKeyPath);
            var execPath = Environment.ProcessPath ?? "KwmSwitcher";
            key.SetValue(AppName, $"\"{execPath}\"");
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to enable autostart");
        }
    }

    public void Disable()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(RegistryKeyPath, true);
            if (key?.GetValue(AppName) != null)
                key.DeleteValue(AppName);
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to disable autostart");
        }
    }
}