using System;
using System.IO;
using Serilog;

namespace KwmSwitcher.Services.Linux;

public class LinuxAutoStartService : IAutoStartService
{
    private static readonly string AutoStartDir = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "autostart");

    private static readonly string DesktopFilePath = Path.Combine(AutoStartDir, "KwmSwitcher.desktop");

    private const string DesktopFileContent = """"
[Desktop Entry]
Type=Application
Name=KWM Switcher
Exec={0}
Icon=KwmSwitcher
Comment=USB KVM switcher for monitor input
Hidden=false
NoDisplay=false
X-GNOME-Autostart-enabled=true
"""";

    public bool IsEnabled()
    {
        try
        {
            return File.Exists(DesktopFilePath);
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
            Directory.CreateDirectory(AutoStartDir);
            var execPath = Environment.ProcessPath ?? "KwmSwitcher";
            File.WriteAllText(DesktopFilePath, string.Format(DesktopFileContent, execPath));
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
            if (File.Exists(DesktopFilePath))
                File.Delete(DesktopFilePath);
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to disable autostart");
        }
    }
}