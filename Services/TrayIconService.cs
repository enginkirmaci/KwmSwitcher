using System;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Serilog;

namespace KwmSwitcher.Services;

/// <summary>
/// Builds the tray icon + menu and routes clicks to host-supplied actions.
/// Extracted from <c>App</c> so the composition root stays focused on wiring.
/// </summary>
public sealed class TrayIconService : ITrayIconService
{
    private TrayIcon? _trayIcon;

    public void Initialize(
        Action showMainWindow,
        Action openSettings,
        Func<Task> switchToLocal,
        Func<Task> switchToRemote,
        Func<Task> togglePip,
        Action quit)
    {
        var menu = new NativeMenu();
        menu.Items.Add(MenuItem("Main", showMainWindow));
        menu.Items.Add(MenuItem("Settings", openSettings));
        menu.Items.Add(new NativeMenuItemSeparator());
        menu.Items.Add(AsyncMenuItem("Switch to Local", switchToLocal, "switching to local"));
        menu.Items.Add(AsyncMenuItem("Switch to Remote", switchToRemote, "switching to remote"));
        menu.Items.Add(AsyncMenuItem("Toggle PiP/PBP", togglePip, "toggling PiP"));
        menu.Items.Add(new NativeMenuItemSeparator());
        menu.Items.Add(MenuItem("Quit", quit));

        _trayIcon = new TrayIcon
        {
            Icon = new WindowIcon(new Bitmap(AssetLoader.Open(new Uri("avares://KwmSwitcher/Assets/app.ico")))),
            ToolTipText = "KWM Switcher",
            Menu = menu,
        };

        var app = Application.Current;
        if (app == null) return;

        var icons = TrayIcon.GetIcons(app);
        if (icons == null)
        {
            icons = [];
            TrayIcon.SetIcons(app, icons);
        }

        icons.Add(_trayIcon);
    }

    /// <summary>Builds a synchronous menu item that runs <paramref name="action"/> on click.</summary>
    private static NativeMenuItem MenuItem(string header, Action action)
    {
        var item = new NativeMenuItem(header);
        item.Click += (_, _) => action();
        return item;
    }

    /// <summary>
    /// Builds an async menu item whose click handler awaits
    /// <paramref name="action"/> and logs any exception with the supplied
    /// <paramref name="errorLabel"/> — the shared shape of the three tray
    /// operations (switch local / remote / toggle PiP).
    /// </summary>
    private static NativeMenuItem AsyncMenuItem(string header, Func<Task> action, string errorLabel)
    {
        var item = new NativeMenuItem(header);
        item.Click += async (_, _) =>
        {
            try { await action(); }
            catch (Exception ex) { Log.Error(ex, "Error {Label} from tray", errorLabel); }
        };
        return item;
    }

    public void Dispose()
    {
        _trayIcon?.Dispose();
        _trayIcon = null;
    }
}
