using System;
using System.Collections.Generic;
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
    private readonly IAutoStartService _autoStartService;

    public event Action? RequestClose;

    [ObservableProperty]
    private ObservableCollection<UsbDeviceItem> _availableDevices = [];

    public ObservableCollection<InputProtocolOption> InputProtocols { get; } =
    [
        new(InputSwitchProtocol.Standard, "Standard DDC/CI (0x60)"),
        new(InputSwitchProtocol.Lg, "LG (0xF4)"),
    ];

    [ObservableProperty]
    private InputProtocolOption _selectedInputProtocol;

    [ObservableProperty]
    private ObservableCollection<InputSourceOption> _inputSources = [];

    [ObservableProperty]
    private InputSourceOption _selectedLocalInput;

    [ObservableProperty]
    private InputSourceOption _selectedRemoteInput;

    [ObservableProperty]
    private bool _startMinimized;

    [ObservableProperty]
    private bool _autoStart;

    [ObservableProperty]
    private ObservableCollection<string> _availableMonitors = [];

    [ObservableProperty]
    private string? _selectedTargetMonitor;

    partial void OnSelectedInputProtocolChanged(InputProtocolOption value)
    {
        UpdateInputSources(value.Protocol);
    }

    public SettingsViewModel(IUsbMonitor usbMonitor, AppConfig config, IAutoStartService autoStartService, IReadOnlyList<string>? availableMonitors = null)
    {
        _usbMonitor = usbMonitor;
        _config = config;
        _autoStartService = autoStartService;

        AvailableMonitors = availableMonitors != null
            ? new ObservableCollection<string>(availableMonitors)
            : [];
        SelectedTargetMonitor = !string.IsNullOrWhiteSpace(_config.TargetMonitorName) &&
                                AvailableMonitors.Contains(_config.TargetMonitorName!)
            ? _config.TargetMonitorName
            : AvailableMonitors.FirstOrDefault();

        SelectedInputProtocol = InputProtocols.First(p => p.Protocol == _config.InputProtocol);
        UpdateInputSources(_config.InputProtocol);

        SelectedLocalInput = InputSources.FirstOrDefault(i => i.Code == _config.LocalInputSource)
                             ?? InputSources.First();
        SelectedRemoteInput = InputSources.FirstOrDefault(i => i.Code == _config.RemoteInputSource)
                              ?? InputSources.Skip(1).FirstOrDefault()
                              ?? InputSources.First();
        StartMinimized = _config.StartMinimized;
        AutoStart = _autoStartService.IsEnabled();

        RefreshDevices();
    }

    private void UpdateInputSources(InputSwitchProtocol protocol)
    {
        InputSources = protocol switch
        {
            InputSwitchProtocol.Lg => new ObservableCollection<InputSourceOption>(
            [
                new(MonitorInputSource.DisplayPort, "DisplayPort"),
                new(MonitorInputSource.UsbC, "USB-C"),
                new(MonitorInputSource.Hdmi1, "HDMI-1"),
                new(MonitorInputSource.Hdmi2, "HDMI-2"),
            ]),
            _ => new ObservableCollection<InputSourceOption>(
            [
                new(MonitorInputSource.DisplayPort, "DisplayPort"),
                new(MonitorInputSource.Hdmi1, "HDMI-1"),
                new(MonitorInputSource.Hdmi2, "HDMI-2"),
                new(MonitorInputSource.Dvi, "DVI"),
                new(MonitorInputSource.Vga, "VGA"),
            ])
        };

        // Ensure selections are valid for the new source list.
        SelectedLocalInput = InputSources.FirstOrDefault(i => i.Code == _config.LocalInputSource)
                             ?? InputSources.First();
        SelectedRemoteInput = InputSources.FirstOrDefault(i => i.Code == _config.RemoteInputSource)
                              ?? InputSources.Skip(1).FirstOrDefault()
                              ?? InputSources.First();
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
        _config.InputProtocol = SelectedInputProtocol.Protocol;
        _config.LocalInputSource = SelectedLocalInput.Code;
        _config.RemoteInputSource = SelectedRemoteInput.Code;
        _config.StartMinimized = StartMinimized;
        _config.AutoStart = AutoStart;
        if (AutoStart)
            _autoStartService.Enable();
        else
            _autoStartService.Disable();
        _config.TargetMonitorName = SelectedTargetMonitor;
        _config.TrackedDeviceKeys = AvailableDevices
            .Where(d => d.IsTracked)
            .Select(d => d.Key)
            .ToList();
        _config.Save();
        RequestClose?.Invoke();
    }

    [RelayCommand]
    private void Cancel()
    {
        RequestClose?.Invoke();
    }
}

public record InputSourceOption(byte Code, string Name);
public record InputProtocolOption(InputSwitchProtocol Protocol, string Name);

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
