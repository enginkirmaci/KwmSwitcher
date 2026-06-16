using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
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
using Serilog;

namespace KwmSwitcher;

public partial class App : Application
{
    private SwitcherEngine? _engine;
    private MainWindowViewModel? _mainViewModel;
    private AppConfig _config = new();
    private IUsbMonitor? _usbMonitor;
    private IMonitorSwitcher? _monitorSwitcher;
    private IAutoStartService? _autoStartService;

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
                _autoStartService = new LinuxAutoStartService();
            }
            else if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            {
                _usbMonitor = new WindowsUsbMonitor();
                _monitorSwitcher = new WindowsMonitorSwitcher(_config);
                _autoStartService = new WindowsAutoStartService();
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

            if (_config.StartMinimized)
            {
                void OnOpened(object? s, EventArgs e)
                {
                    mainWindow.Opened -= OnOpened;
                    mainWindow.Hide();
                }
                mainWindow.Opened += OnOpened;
            }
            else
            {
                mainWindow.Show();
            }

            _mainViewModel.StartEngine();

            desktop.Exit += OnExit;

            SetupTrayIcon(desktop, mainWindow);
        }

        base.OnFrameworkInitializationCompleted();
    }

    private void OnExit(object? sender, ControlledApplicationLifetimeExitEventArgs e)
    {
        ShutdownEngine();
    }

    private void ShutdownEngine()
    {
        try
        {
            _engine?.Dispose();
            _usbMonitor?.Dispose();
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Error during engine shutdown");
        }
    }

    private void SetupTrayIcon(IClassicDesktopStyleApplicationLifetime desktop, MainWindow mainWindow)
    {
        var showItem = new NativeMenuItem("Main");
        showItem.Click += (_, _) => mainWindow.Show();

        var settingsItem = new NativeMenuItem("Settings");
        settingsItem.Click += (_, _) => OpenSettings(mainWindow);

        var switchLocalItem = new NativeMenuItem("Switch to Local");
        switchLocalItem.Click += async (_, _) =>
        {
            try { if (_engine != null) await _engine.SwitchToLocalAsync(); }
            catch (Exception ex) { Log.Error(ex, "Error switching to local from tray"); }
        };

        var switchRemoteItem = new NativeMenuItem("Switch to Remote");
        switchRemoteItem.Click += async (_, _) =>
        {
            try { if (_engine != null) await _engine.SwitchToRemoteAsync(); }
            catch (Exception ex) { Log.Error(ex, "Error switching to remote from tray"); }
        };

        var quitItem = new NativeMenuItem("Quit");
        quitItem.Click += (_, _) =>
        {
            ShutdownEngine();
            desktop.Shutdown();
        };

        var menu = new NativeMenu();
        menu.Items.Add(showItem);
        menu.Items.Add(settingsItem);
        menu.Items.Add(new NativeMenuItemSeparator());
        menu.Items.Add(switchLocalItem);
        menu.Items.Add(switchRemoteItem);
        menu.Items.Add(new NativeMenuItemSeparator());
        menu.Items.Add(quitItem);

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

        var settingsVm = new SettingsViewModel(_usbMonitor, _config, _autoStartService!, availableMonitors);
        var settingsWindow = new SettingsWindow
        {
            DataContext = settingsVm,
        };

        settingsVm.RequestClose += () => settingsWindow.Close();

        settingsWindow.Show();
    }
}