using System.Collections.Generic;
using System.Threading.Tasks;

namespace KwmSwitcher.Services;

public interface IMonitorSwitcher
{
    Task<bool> SetInputSourceAsync(byte inputSource);
    Task<byte> GetInputSourceAsync();
    Task<byte> GetPipModeAsync();
    Task<bool> SetPipModeAsync(byte mode);

    /// <summary>
    /// Lists human-readable descriptions of currently connected monitors.
    /// Used to populate the target-monitor picker in settings. Returns an
    /// empty list on platforms/backends where enumeration is unsupported.
    /// </summary>
    Task<IReadOnlyList<string>> GetAvailableMonitorsAsync();
}

