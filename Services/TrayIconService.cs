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
        var showItem = new NativeMenuItem("Main");
        showItem.Click += (_, _) => showMainWindow();

        var settingsItem = new NativeMenuItem("Settings");
        settingsItem.Click += (_, _) => openSettings();

        var switchLocalItem = new NativeMenuItem("Switch to Local");
        switchLocalItem.Click += async (_, _) =>
        {
            try { await switchToLocal(); }
            catch (Exception ex) { Log.Error(ex, "Error switching to local from tray"); }
        };

        var switchRemoteItem = new NativeMenuItem("Switch to Remote");
        switchRemoteItem.Click += async (_, _) =>
        {
            try { await switchToRemote(); }
            catch (Exception ex) { Log.Error(ex, "Error switching to remote from tray"); }
        };

        var togglePipItem = new NativeMenuItem("Toggle PiP/PBP");
        togglePipItem.Click += async (_, _) =>
        {
            try { await togglePip(); }
            catch (Exception ex) { Log.Error(ex, "Error toggling PiP from tray"); }
        };

        var quitItem = new NativeMenuItem("Quit");
        quitItem.Click += (_, _) => quit();

        var menu = new NativeMenu();
        menu.Items.Add(showItem);
        menu.Items.Add(settingsItem);
        menu.Items.Add(new NativeMenuItemSeparator());
        menu.Items.Add(switchLocalItem);
        menu.Items.Add(switchRemoteItem);
        menu.Items.Add(togglePipItem);
        menu.Items.Add(new NativeMenuItemSeparator());
        menu.Items.Add(quitItem);

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

    public void Dispose()
    {
        _trayIcon?.Dispose();
        _trayIcon = null;
    }
}
