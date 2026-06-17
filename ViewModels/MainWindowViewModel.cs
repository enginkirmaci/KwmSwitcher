using System;
using System.Collections.ObjectModel;
using System.Threading.Tasks;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using KwmSwitcher.Models;
using KwmSwitcher.Services;

namespace KwmSwitcher.ViewModels;

public partial class MainWindowViewModel : ViewModelBase
{
    private readonly SwitcherEngine? _engine;
    private readonly AppConfig _config;

    [ObservableProperty]
    private string _statusText = "Initializing...";

    [ObservableProperty]
    private bool _isLocalActive;

    [ObservableProperty]
    private bool _isRemoteActive;

    [ObservableProperty]
    private string _activeLabel = "Unknown";

    [ObservableProperty]
    private bool _isPipActive;

    [ObservableProperty]
    private string _pipLabel = "PiP";

    [ObservableProperty]
    private bool _isPipBusy;

    public event Action? ShowSettingsRequested;

    // Design-time only: satisfies the XAML <Design.DataContext> instance.
    // Runtime construction must use the (SwitcherEngine, AppConfig) overload.
    public MainWindowViewModel()
    {
        _config = AppConfig.Load();
    }

    public MainWindowViewModel(SwitcherEngine engine, AppConfig config) : this()
    {
        _engine = engine;
        _config = config;

        // The engine raises these on the UI thread via its postToConsumer hook,
        // so handlers can update bound properties directly.
        _engine.LocalActiveChanged += active =>
        {
            IsLocalActive = active;
            IsRemoteActive = !active;
            ActiveLabel = active ? "Local (this machine)" : "Remote (other machine)";
        };

        _engine.StatusChanged += status =>
        {
            StatusText = status;
        };

        _engine.PipModeChanged += mode =>
        {
            IsPipActive = MonitorInputSource.IsPipActive(mode);
            PipLabel = IsPipActive ? MonitorInputSource.GetPipModeName(mode) : "PiP";
        };
    }

    public void StartEngine()
    {
        _engine?.Start();
    }

    [RelayCommand]
    private async Task SwitchToLocal()
    {
        if (_engine != null)
            await _engine.SwitchToLocalAsync();
    }

    [RelayCommand]
    private async Task SwitchToRemote()
    {
        if (_engine != null)
            await _engine.SwitchToRemoteAsync();
    }

    [RelayCommand]
    private async Task TogglePip()
    {
        if (_engine == null) return;
        IsPipBusy = true;
        try
        {
            await _engine.TogglePipAsync();
        }
        finally
        {
            Dispatcher.UIThread.Post(() => IsPipBusy = false);
        }
    }

    [RelayCommand]
    private async Task RefreshPip()
    {
        if (_engine != null)
            await _engine.RefreshPipStateAsync();
    }

    [RelayCommand]
    private void OpenSettings()
    {
        ShowSettingsRequested?.Invoke();
    }
}
