using System.Runtime.Versioning;
using Microsoft.Win32;

namespace KwmSwitcher.Services.Windows;

/// <summary>
/// Windows autostart via the <c>Run</c> registry key under HKCU. Error handling
/// (try/catch + logging) is provided by <see cref="AutoStartServiceBase"/>; this
/// class implements only the registry mechanics.
/// </summary>
[SupportedOSPlatform("windows")]
public class WindowsAutoStartService : AutoStartServiceBase
{
    private const string RegistryKeyPath = @"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    private const string AppName = "KwmSwitcher";

    protected override bool IsEnabledCore()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RegistryKeyPath, false);
        return key?.GetValue(AppName) != null;
    }

    protected override void EnableCore()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RegistryKeyPath, true)
                     ?? Registry.CurrentUser.CreateSubKey(RegistryKeyPath);
        var execPath = System.Environment.ProcessPath ?? "KwmSwitcher";
        key.SetValue(AppName, $"\"{execPath}\"");
    }

    protected override void DisableCore()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RegistryKeyPath, true);
        if (key?.GetValue(AppName) != null)
            key.DeleteValue(AppName);
    }
}
