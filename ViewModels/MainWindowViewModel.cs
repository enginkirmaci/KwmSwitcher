using System.Collections.ObjectModel;
using System.Threading.Tasks;
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
    private string _activeLabel = "Unknown";

    public MainWindowViewModel()
    {
        _config = AppConfig.Load();
    }

    public MainWindowViewModel(SwitcherEngine engine, AppConfig config) : this()
    {
        _engine = engine;
        _config = config;

        _engine.LocalActiveChanged += active =>
        {
            IsLocalActive = active;
            ActiveLabel = active ? "Local (this machine)" : "Remote (other machine)";
        };

        _engine.StatusChanged += status =>
        {
            StatusText = status;
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
}
