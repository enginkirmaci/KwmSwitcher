using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using KwmSwitcher.Models;
using KwmSwitcher.Services;
using KwmSwitcher.Services.Linux;
using KwmSwitcher.Services.Windows;
using KwmSwitcher.ViewModels;
using KwmSwitcher.Views;

namespace KwmSwitcher;

public partial class App : Application
{
    private SwitcherEngine? _engine;
    private MainWindowViewModel? _mainViewModel;
    private AppConfig _config = new();
    private IUsbMonitor? _usbMonitor;
    private IMonitorSwitcher? _monitorSwitcher;

    public override void Initialize()
    {
        AvaloniaXamlLoader.Load(this);
    }

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            _config = AppConfig.Load();

            if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
            {
                _usbMonitor = new LinuxUsbMonitor();
                _monitorSwitcher = new LinuxMonitorSwitcher(_config);
            }
            else if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            {
                _usbMonitor = new WindowsUsbMonitor();
                _monitorSwitcher = new WindowsMonitorSwitcher(_config);
            }
            else
            {
                Console.Error.WriteLine("Unsupported platform");
                desktop.Shutdown(1);
                return;
            }

            _engine = new SwitcherEngine(_usbMonitor, _monitorSwitcher, _config);
            _mainViewModel = new MainWindowViewModel(_engine, _config);

            var mainWindow = new MainWindow
            {
                DataContext = _mainViewModel,
            };

            _mainViewModel.ShowSettingsRequested += () => OpenSettings(mainWindow);

            mainWindow.Closing += (_, e) =>
            {
                e.Cancel = true;
                mainWindow.Hide();
            };

            desktop.MainWindow = mainWindow;

            if (!_config.StartMinimized)
                mainWindow.Show();

            _mainViewModel.StartEngine();

            SetupTrayIcon(desktop, mainWindow);
        }

        base.OnFrameworkInitializationCompleted();
    }

    private void SetupTrayIcon(IClassicDesktopStyleApplicationLifetime desktop, MainWindow mainWindow)
    {
        var showItem = new NativeMenuItem("Show");
        showItem.Click += (_, _) => mainWindow.Show();

        var settingsItem = new NativeMenuItem("Settings");
        settingsItem.Click += (_, _) => OpenSettings(mainWindow);

        var switchLocalItem = new NativeMenuItem("Switch to Local");
        switchLocalItem.Click += async (_, _) =>
        {
            if (_engine != null)
                await _engine.SwitchToLocalAsync();
        };

        var switchRemoteItem = new NativeMenuItem("Switch to Remote");
        switchRemoteItem.Click += async (_, _) =>
        {
            if (_engine != null)
                await _engine.SwitchToRemoteAsync();
        };

        var quitItem = new NativeMenuItem("Quit");
        quitItem.Click += (_, _) =>
        {
            _engine?.Stop();
            _usbMonitor?.Dispose();
            desktop.Shutdown();
        };

        var menu = new NativeMenu
        {
            showItem,
            settingsItem,
            new NativeMenuItemSeparator(),
            switchLocalItem,
            switchRemoteItem,
            new NativeMenuItemSeparator(),
            quitItem,
        };

        var trayIcon = new TrayIcon
        {
            Icon = new WindowIcon(new Bitmap(AssetLoader.Open(new Uri("avares://KwmSwitcher/Assets/avalonia-logo.ico")))),
            ToolTipText = "KWM Switcher",
            Menu = menu,
        };

        var icons = TrayIcon.GetIcons(this);
        if (icons == null)
        {
            icons = [];
            TrayIcon.SetIcons(this, icons);
        }

        icons.Add(trayIcon);
    }

    private void OpenSettings(Window owner)
    {
        if (_usbMonitor == null)
            return;

        IReadOnlyList<string>? availableMonitors = null;
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            availableMonitors = WindowsMonitorSwitcher.GetAvailableMonitorDescriptions();
        }

        var settingsVm = new SettingsViewModel(_usbMonitor, _config, availableMonitors);
        var settingsWindow = new SettingsWindow
        {
            DataContext = settingsVm,
        };

        settingsVm.RequestClose += () => settingsWindow.Close();

        settingsWindow.Show(owner);
    }
}
