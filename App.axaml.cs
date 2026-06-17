using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml;
using Avalonia.Threading;
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
    private ITrayIconService? _trayIconService;

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
                _usbMonitor = new LinuxUsbMonitor(_config);
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

            _engine = new SwitcherEngine(_usbMonitor, _monitorSwitcher, _config,
                postToConsumer: action => Dispatcher.UIThread.Post(action));
            _mainViewModel = new MainWindowViewModel(_engine, _config);

            var mainWindow = new MainWindow
            {
                DataContext = _mainViewModel,
            };

            _mainViewModel.ShowSettingsRequested += () => _ = OpenSettingsAsync(mainWindow);

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

            _trayIconService = new TrayIconService();
            _trayIconService.Initialize(
                showMainWindow: mainWindow.Show,
                openSettings: () => _ = OpenSettingsAsync(mainWindow),
                switchToLocal: () => SafeSwitchAsync(() => _engine!.SwitchToLocalAsync(), "local"),
                switchToRemote: () => SafeSwitchAsync(() => _engine!.SwitchToRemoteAsync(), "remote"),
                togglePip: () => SafeSwitchAsync(() => _engine!.TogglePipAsync(), "toggle PiP"),
                quit: () => { ShutdownEngine(); desktop.Shutdown(); });
        }

        base.OnFrameworkInitializationCompleted();
    }

    private static async Task SafeSwitchAsync(Func<Task> action, string label)
    {
        try { await action(); }
        catch (Exception ex) { Log.Error(ex, "Error {Label} from tray", label); }
    }

    private void OnExit(object? sender, ControlledApplicationLifetimeExitEventArgs e)
    {
        ShutdownEngine();
    }

    private void ShutdownEngine()
    {
        try
        {
            _trayIconService?.Dispose();
            _engine?.Dispose();
            _usbMonitor?.Dispose();
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Error during engine shutdown");
        }
    }

    private async Task OpenSettingsAsync(Window owner)
    {
        if (_usbMonitor == null || _monitorSwitcher == null)
            return;

        IReadOnlyList<string> availableMonitors = await _monitorSwitcher.GetAvailableMonitorsAsync();

        var settingsVm = new SettingsViewModel(_usbMonitor, _config, _autoStartService!, availableMonitors);
        var settingsWindow = new SettingsWindow
        {
            DataContext = settingsVm,
        };

        settingsVm.RequestClose += () => settingsWindow.Close();

        settingsWindow.Show();
    }
}