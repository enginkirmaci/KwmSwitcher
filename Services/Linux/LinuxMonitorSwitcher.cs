using System;
using System.Diagnostics;
using System.Globalization;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using KwmSwitcher.Models;
using Serilog;

namespace KwmSwitcher.Services.Linux;

public partial class LinuxMonitorSwitcher : IMonitorSwitcher
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
            var args = BuildSetVcpArgs(_config.InputProtocol, vcpCode, value,
                MonitorInputSource.GetInputI2cSourceAddress(_config.InputProtocol));

            var (success, stderr) = await RunDdcutilAsync(args);
            if (!success && !string.IsNullOrWhiteSpace(stderr))
            {
                Log.Warning("ddcutil setvcp failed: {Stderr}", stderr.Trim());
                Console.Error.WriteLine($"ddcutil setvcp failed: {stderr.Trim()}");
            }
            return success;
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to set input source");
            Console.Error.WriteLine($"Failed to set input source: {ex.Message}");
            return false;
        }
    }

    public async Task<byte> GetInputSourceAsync()
    {
        try
        {
            var vcpCode = MonitorInputSource.GetVcpCode(_config.InputProtocol);
            var args = BuildGetVcpArgs(_config.InputProtocol, vcpCode,
                MonitorInputSource.GetInputI2cSourceAddress(_config.InputProtocol));

            var (success, stdout, stderr) = await RunDdcutilCaptureAsync(args);
            if (!success)
            {
                if (!string.IsNullOrWhiteSpace(stderr))
                    Log.Warning("ddcutil getvcp failed: {Stderr}", stderr.Trim());
                return 0;
            }

            var parsed = TryParseIncomingValue(stdout);
            if (!parsed.HasValue)
            {
                Log.Warning("ddcutil getvcp returned unparsable output: {Stdout}", stdout.Trim());
                return 0;
            }

            return MonitorInputSource.DecodeInputSource(_config.InputProtocol, parsed.Value);
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to get input source");
            Console.Error.WriteLine($"Failed to get input source: {ex.Message}");
            return 0;
        }
    }

    public async Task<byte> GetPipModeAsync()
    {
        try
        {
            var vcpCode = MonitorInputSource.GetPipVcpCode(_config.InputProtocol);
            var args = BuildGetVcpArgs(_config.InputProtocol, vcpCode,
                MonitorInputSource.GetPipI2cSourceAddress(_config.InputProtocol));

            var (success, stdout, stderr) = await RunDdcutilCaptureAsync(args);
            if (!success)
            {
                if (!string.IsNullOrWhiteSpace(stderr))
                    Log.Warning("ddcutil getvcp PiP failed: {Stderr}", stderr.Trim());
                return 0;
            }

            var parsed = TryParseIncomingValue(stdout);
            if (!parsed.HasValue)
            {
                Log.Warning("ddcutil getvcp PiP returned unparsable output: {Stdout}", stdout.Trim());
                return 0;
            }

            return MonitorInputSource.DecodePipMode(_config.InputProtocol, parsed.Value);
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to get PiP mode");
            return 0;
        }
    }

    public async Task<bool> SetPipModeAsync(byte mode)
    {
        try
        {
            var vcpCode = MonitorInputSource.GetPipVcpCode(_config.InputProtocol);
            var value = MonitorInputSource.GetPipProtocolValue(_config.InputProtocol, mode);
            var args = BuildSetVcpArgs(_config.InputProtocol, vcpCode, value,
                MonitorInputSource.GetPipI2cSourceAddress(_config.InputProtocol));

            var (success, stderr) = await RunDdcutilAsync(args);
            if (!success && !string.IsNullOrWhiteSpace(stderr))
            {
                Log.Warning("ddcutil setvcp PiP failed: {Stderr}", stderr.Trim());
                Console.Error.WriteLine($"ddcutil setvcp PiP failed: {stderr.Trim()}");
            }
            return success;
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to set PiP mode");
            Console.Error.WriteLine($"Failed to set PiP mode: {ex.Message}");
            return false;
        }
    }

    private static string BuildSetVcpArgs(InputSwitchProtocol protocol, byte vcpCode, byte value, byte i2cAddr) =>
        protocol == InputSwitchProtocol.Lg && i2cAddr != 0
            ? $"--i2c-source-addr=0x{i2cAddr:X2} setvcp 0x{vcpCode:X2} 0x{value:X2} --noverify"
            : $"setvcp 0x{vcpCode:X2} 0x{value:X2}";

    private static string BuildGetVcpArgs(InputSwitchProtocol protocol, byte vcpCode, byte i2cAddr) =>
        protocol == InputSwitchProtocol.Lg && i2cAddr != 0
            ? $"--i2c-source-addr=0x{i2cAddr:X2} getvcp 0x{vcpCode:X2}"
            : $"getvcp 0x{vcpCode:X2}";

    private static byte? TryParseIncomingValue(string output)
    {
        if (string.IsNullOrWhiteSpace(output))
            return null;

        var match = IncomingValueRegex().Match(output);
        if (!match.Success)
            return null;

        var hex = match.Groups[1].Value;
        if (byte.TryParse(hex,
                NumberStyles.HexNumber,
                CultureInfo.InvariantCulture,
                out var value))
        {
            return value;
        }

        return null;
    }

    [GeneratedRegex(@"Incoming\s*[=:]\s*(0x[0-9A-Fa-f]+)",
        RegexOptions.IgnoreCase, "en-US")]
    private static partial Regex IncomingValueRegex();

    private static async Task<(bool Success, string Stdout, string Stderr)> RunDdcutilCaptureAsync(string arguments)
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
            return (false, "", "Failed to start ddcutil process");

        var stdoutTask = process.StandardOutput.ReadToEndAsync();
        var stderrTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        return (process.ExitCode == 0, await stdoutTask, await stderrTask);
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
