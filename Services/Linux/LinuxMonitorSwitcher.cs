using System;
using System.Diagnostics;
using System.Threading.Tasks;
using KwmSwitcher.Models;

namespace KwmSwitcher.Services.Linux;

public class LinuxMonitorSwitcher : IMonitorSwitcher
{
    private readonly AppConfig _config;

    public LinuxMonitorSwitcher(AppConfig config)
    {
        _config = config;
    }

    public async Task<bool> SetInputSourceAsync(byte inputSource)
    {
        try
        {
            var vcpCode = MonitorInputSource.GetVcpCode(_config.InputProtocol);
            var value = MonitorInputSource.GetProtocolValue(_config.InputProtocol, inputSource);
            var args = _config.InputProtocol == InputSwitchProtocol.Lg
                ? $"--i2c-source-addr=0x50 setvcp 0x{vcpCode:X2} 0x{value:X2} --noverify"
                : $"setvcp 0x{vcpCode:X2} 0x{value:X2}";

            var (success, stderr) = await RunDdcutilAsync(args);
            if (!success && !string.IsNullOrWhiteSpace(stderr))
            {
                Console.Error.WriteLine($"ddcutil setvcp failed: {stderr.Trim()}");
            }
            return success;
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
            var psi = new ProcessStartInfo("ddcutil", $"getvcp 0x{MonitorInputSource.VcpCode:X2}")
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

    private static async Task<(bool Success, string Stderr)> RunDdcutilAsync(string arguments)
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
            return (false, "Failed to start ddcutil process");

        var stderrTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        var stderr = await stderrTask;

        return (process.ExitCode == 0, stderr);
    }
}
