using System.Threading.Tasks;

namespace KwmSwitcher.Services;

public interface IMonitorSwitcher
{
    Task<bool> SetInputSourceAsync(byte inputSource);
    Task<byte> GetInputSourceAsync();
}
