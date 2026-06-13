using System;
using System.Collections.ObjectModel;
using System.Linq;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using KwmSwitcher.Models;
using KwmSwitcher.Services;

namespace KwmSwitcher.ViewModels;

public partial class SettingsViewModel : ViewModelBase
{
    private readonly IUsbMonitor _usbMonitor;
    private readonly AppConfig _config;

    public event Action? RequestClose;

    [ObservableProperty]
    private ObservableCollection<UsbDeviceItem> _availableDevices = [];

    public ObservableCollection<InputSourceOption> InputSources { get; } =
    [
        new(MonitorInputSource.DisplayPort, "DisplayPort"),
        new(MonitorInputSource.Hdmi1, "HDMI-1"),
        new(MonitorInputSource.Hdmi2, "HDMI-2"),
        new(MonitorInputSource.Dvi, "DVI"),
        new(MonitorInputSource.Vga, "VGA"),
    ];

    [ObservableProperty]
    private InputSourceOption _selectedLocalInput;

    [ObservableProperty]
    private InputSourceOption _selectedRemoteInput;

    [ObservableProperty]
    private bool _startMinimized;

    public SettingsViewModel(IUsbMonitor usbMonitor, AppConfig config)
    {
        _usbMonitor = usbMonitor;
        _config = config;

        SelectedLocalInput = InputSources.First(i => i.Code == _config.LocalInputSource);
        SelectedRemoteInput = InputSources.First(i => i.Code == _config.RemoteInputSource);
        StartMinimized = _config.StartMinimized;

        RefreshDevices();
    }

    [RelayCommand]
    private void RefreshDevices()
    {
        var trackedKeys = _config.TrackedDeviceKeys.ToHashSet();
        var devices = _usbMonitor.GetCurrentDevices();

        AvailableDevices = new ObservableCollection<UsbDeviceItem>(
            devices.Select(d => new UsbDeviceItem(d)
            {
                IsTracked = trackedKeys.Contains(d.Key)
            }));
    }

    [RelayCommand]
    private void Save()
    {
        _config.LocalInputSource = SelectedLocalInput.Code;
        _config.RemoteInputSource = SelectedRemoteInput.Code;
        _config.StartMinimized = StartMinimized;
        _config.TrackedDeviceKeys = AvailableDevices
            .Where(d => d.IsTracked)
            .Select(d => d.Key)
            .ToList();
        _config.Save();
        RequestClose?.Invoke();
    }
}

public record InputSourceOption(byte Code, string Name);

public partial class UsbDeviceItem : ViewModelBase
{
    public UsbDeviceItem(UsbDeviceInfo info)
    {
        Info = info;
        Key = info.Key;
        Description = info.Description;
    }

    public UsbDeviceInfo Info { get; }
    public string Key { get; }
    public string Description { get; }

    [ObservableProperty]
    private bool _isTracked;
}
