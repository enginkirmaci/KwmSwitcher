using System;
using System.Diagnostics;
using System.Threading.Tasks;

namespace KwmSwitcher.Services.Linux;

public class LinuxMonitorSwitcher : IMonitorSwitcher
{
    public async Task<bool> SetInputSourceAsync(byte inputSource)
    {
        try
        {
            var result = await RunDdcutilAsync(
                $"setvcp 0x{Models.MonitorInputSource.VcpCode:X2} 0x{inputSource:X2}");
            return result;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Failed to set input source: {ex.Message}");
            return false;
        }
    }

    public async Task<byte> GetInputSourceAsync()
    {
        try
        {
            var psi = new ProcessStartInfo("ddcutil", $"getvcp 0x{Models.MonitorInputSource.VcpCode:X2}")
            {
                RedirectStandardOutput = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };

            using var process = Process.Start(psi);
            if (process == null)
                return 0;

            var output = await process.StandardOutput.ReadToEndAsync();
            await process.WaitForExitAsync();

            var parts = output.Split("=", StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length >= 2)
            {
                var valueStr = parts[^1].Trim().Split(' ')[0].Trim();
                if (valueStr.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
                    return Convert.ToByte(valueStr, 16);
            }

            return 0;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Failed to get input source: {ex.Message}");
            return 0;
        }
    }

    private static async Task<bool> RunDdcutilAsync(string arguments)
    {
        var psi = new ProcessStartInfo("ddcutil", arguments)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true
        };

        using var process = Process.Start(psi);
        if (process == null)
            return false;

        await process.WaitForExitAsync();
        return process.ExitCode == 0;
    }
}
