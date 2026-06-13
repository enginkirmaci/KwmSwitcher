using System;
using System.Collections.Generic;
using KwmSwitcher.Models;

namespace KwmSwitcher.Services;

public interface IUsbMonitor : IDisposable
{
    event Action<IEnumerable<UsbDeviceInfo>>? DevicesChanged;
    IReadOnlyList<UsbDeviceInfo> GetCurrentDevices();
    void Start();
    void Stop();
}
