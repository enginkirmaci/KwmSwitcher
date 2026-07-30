using System.IO;

namespace KwmSwitcher.Services.Linux;

/// <summary>
/// Linux autostart via a FreeDesktop <c>.desktop</c> file in the user's
/// <c>autostart</c> directory. Error handling (try/catch + logging) is provided
/// by <see cref="AutoStartServiceBase"/>; this class implements only the
/// filesystem mechanics.
/// </summary>
public class LinuxAutoStartService : AutoStartServiceBase
{
    private static readonly string AutoStartDir = Path.Combine(
        System.Environment.GetFolderPath(System.Environment.SpecialFolder.ApplicationData),
        "autostart");

    private static readonly string DesktopFilePath = Path.Combine(AutoStartDir, "KwmSwitcher.desktop");

    private const string DesktopFileContent = """"
[Desktop Entry]
Type=Application
Name=KWM Switcher
Exec={0} --supervise
Icon=KwmSwitcher
Comment=USB KVM switcher for monitor input
Hidden=false
NoDisplay=false
X-GNOME-Autostart-enabled=true
"""";

    protected override bool IsEnabledCore() => File.Exists(DesktopFilePath);

    protected override void EnableCore()
    {
        Directory.CreateDirectory(AutoStartDir);
        var execPath = System.Environment.ProcessPath ?? "KwmSwitcher";
        File.WriteAllText(DesktopFilePath, string.Format(DesktopFileContent, execPath));
    }

    protected override void DisableCore()
    {
        if (File.Exists(DesktopFilePath))
            File.Delete(DesktopFilePath);
    }
}
