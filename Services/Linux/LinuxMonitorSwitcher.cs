using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using KwmSwitcher.Models;
using Serilog;

namespace KwmSwitcher.Services.Linux;

public partial class LinuxMonitorSwitcher : IMonitorSwitcher
{
    private readonly AppConfig _config;

    /// <summary>
    /// Serializes every <c>ddcutil</c> invocation. DDC/CI talks to a single
    /// i2c bus; concurrent getvcp/setvcp/detect calls contend for it and are
    /// the most plausible cause of native i2c faults when the monitor is
    /// mid-transition (e.g. right after a KVM switch). This lock guarantees
    /// only one ddcutil process runs at a time.
    /// </summary>
    private readonly SemaphoreSlim _ddcutilBusLock = new(1, 1);

    /// <summary>
    /// Hard deadline for any single ddcutil call. ddcutil normally completes
    /// in well under 10s; this reaps wedged processes (wedged i2c bus after
    /// monitor unplug/replug) instead of letting the await hang forever.
    /// </summary>
    private static readonly TimeSpan DdcutilTimeout = TimeSpan.FromSeconds(15);

    public LinuxMonitorSwitcher(AppConfig config)
    {
        _config = config;
    }

    public Task<bool> SetInputSourceAsync(byte inputSource)
        => SetVcpAsync("input source",
            MonitorInputSource.GetVcpCode(_config.InputProtocol),
            MonitorInputSource.GetProtocolValue(_config.InputProtocol, inputSource),
            MonitorInputSource.GetInputI2cSourceAddress(_config.InputProtocol));

    public Task<byte> GetInputSourceAsync()
        => GetVcpAsync("getvcp",
            MonitorInputSource.GetVcpCode(_config.InputProtocol),
            MonitorInputSource.GetInputI2cSourceAddress(_config.InputProtocol),
            value => MonitorInputSource.DecodeInputSource(_config.InputProtocol, value));

    public Task<byte> GetPipModeAsync()
        => GetVcpAsync("getvcp PiP",
            MonitorInputSource.GetPipVcpCode(_config.InputProtocol),
            MonitorInputSource.GetPipI2cSourceAddress(_config.InputProtocol),
            value => MonitorInputSource.DecodePipMode(_config.InputProtocol, value));

    public Task<bool> SetPipModeAsync(byte mode)
        => SetVcpAsync("setvcp PiP",
            MonitorInputSource.GetPipVcpCode(_config.InputProtocol),
            MonitorInputSource.GetPipProtocolValue(_config.InputProtocol, mode),
            MonitorInputSource.GetPipI2cSourceAddress(_config.InputProtocol));

    /// <summary>
    /// Runs <c>ddcutil setvcp</c> for the given VCP code/value. <paramref name="label"/>
    /// is used in log/error messages to distinguish input-source from PiP calls.
    /// </summary>
    private async Task<bool> SetVcpAsync(string label, byte vcpCode, byte value, byte i2cAddr)
    {
        try
        {
            var args = BuildSetVcpArgs(_config.InputProtocol, vcpCode, value, i2cAddr);
            var (success, stderr) = await RunDdcutilAsync(args);
            if (!success && !string.IsNullOrWhiteSpace(stderr))
            {
                Log.Warning("ddcutil {Label} failed: {Stderr}", label, stderr.Trim());
                Console.Error.WriteLine($"ddcutil {label} failed: {stderr.Trim()}");
            }
            return success;
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to {Label}", label);
            Console.Error.WriteLine($"Failed to {label}: {ex.Message}");
            return false;
        }
    }

    /// <summary>
    /// Runs <c>ddcutil getvcp</c> for the given VCP code, parses the
    /// <c>Incoming</c> value, and applies <paramref name="decode"/>. Returns 0
    /// on any failure. <paramref name="label"/> distinguishes input-source from
    /// PiP calls in log messages.
    /// </summary>
    private async Task<byte> GetVcpAsync(string label, byte vcpCode, byte i2cAddr, Func<byte, byte> decode)
    {
        try
        {
            var args = BuildGetVcpArgs(_config.InputProtocol, vcpCode, i2cAddr);
            var (success, stdout, stderr) = await RunDdcutilCaptureAsync(args);
            if (!success)
            {
                if (!string.IsNullOrWhiteSpace(stderr))
                    Log.Warning("ddcutil {Label} failed: {Stderr}", label, stderr.Trim());
                return 0;
            }

            var parsed = TryParseIncomingValue(stdout);
            if (!parsed.HasValue)
            {
                Log.Warning("ddcutil {Label} returned unparsable output: {Stdout}", label, stdout.Trim());
                return 0;
            }

            return decode(parsed.Value);
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to run ddcutil {Label}", label);
            return 0;
        }
    }

    /// <summary>
    /// Best-effort monitor enumeration via <c>ddcutil detect</c>. Each detected
    /// display contributes a synthetic "Display N — Model" label so the user can
    /// pick one as the target. Returns an empty list on any failure.
    /// </summary>
    public async Task<IReadOnlyList<string>> GetAvailableMonitorsAsync()
    {
        try
        {
            var (success, stdout, stderr) = await RunDdcutilCaptureAsync("detect");
            if (!success)
            {
                if (!string.IsNullOrWhiteSpace(stderr))
                    Log.Warning("ddcutil detect failed: {Stderr}", stderr.Trim());
                return [];
            }
            return ParseDetectDescriptions(stdout);
        }
        catch (Exception ex)
        {
            Log.Error(ex, "Failed to enumerate monitors via ddcutil detect");
            return [];
        }
    }

    private static IReadOnlyList<string> ParseDetectDescriptions(string output)
    {
        // ddcutil detect output looks like:
        //   Display 1
        //      I2C bus:  /dev/i2c-5
        //      Monitor:                 GBT3241
        //      ...
        var descriptions = new List<string>();
        var displayIndex = 0;
        string? model = null;

        foreach (var rawLine in output.Split('\n'))
        {
            var line = rawLine.Trim();
            if (line.StartsWith("Display ", StringComparison.Ordinal))
            {
                if (displayIndex > 0 && !string.IsNullOrWhiteSpace(model))
                    descriptions.Add($"Display {displayIndex} — {model}");
                displayIndex = int.TryParse(line.AsSpan("Display ".Length), out var n) ? n : displayIndex + 1;
                model = null;
                continue;
            }

            const string MonitorPrefix = "Monitor:";
            if (line.StartsWith(MonitorPrefix, StringComparison.Ordinal))
            {
                var value = line[MonitorPrefix.Length..].Trim();
                if (!string.IsNullOrWhiteSpace(value))
                    model = value;
            }
        }

        if (displayIndex > 0 && !string.IsNullOrWhiteSpace(model))
            descriptions.Add($"Display {displayIndex} — {model}");

        return descriptions;
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

    /// <summary>
    /// Runs <c>ddcutil</c> with the given args, capturing stdout and stderr.
    /// Both streams are always redirected so callers can pick whichever they
    /// need (writes discard stdout, reads discard stderr implicitly).
    ///
    /// The call is serialized across all ddcutil invocations via
    /// <see cref="_ddcutilBusLock"/> (the i2c bus can't serve concurrent
    /// requests), bounded by <see cref="DdcutilTimeout"/> so a wedged process
    /// is killed rather than awaited forever, and the child is always reaped
    /// in <c>finally</c> so no orphaned ddcutil survives.
    /// </summary>
    private async Task<(bool Success, string Stdout, string Stderr)> RunDdcutilCaptureAsync(string arguments)
    {
        await _ddcutilBusLock.WaitAsync().ConfigureAwait(false);
        try
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

            using var cts = new CancellationTokenSource(DdcutilTimeout);
            bool timedOut = false;
            try
            {
                await process.WaitForExitAsync(cts.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                timedOut = true;
            }

            // Reap the child (and any helpers it spawned) if it's still alive —
            // covers both the timeout path and any unexpected still-running case.
            if (!process.HasExited)
            {
                try { process.Kill(entireProcessTree: true); }
                catch { /* process may have exited between the check and Kill */ }
            }

            if (timedOut)
            {
                Log.Warning("ddcutil timed out after {Seconds}s and was killed: {Args}",
                    DdcutilTimeout.TotalSeconds, arguments);
                return (false, "", $"ddcutil timed out after {DdcutilTimeout.TotalSeconds}s");
            }

            return (process.ExitCode == 0, await stdoutTask, await stderrTask);
        }
        finally
        {
            _ddcutilBusLock.Release();
        }
    }

    /// <summary>Variant for writes that only need stderr. Delegates to the capture helper.</summary>
    private async Task<(bool Success, string Stderr)> RunDdcutilAsync(string arguments)
    {
        var (success, _, stderr) = await RunDdcutilCaptureAsync(arguments).ConfigureAwait(false);
        return (success, stderr);
    }
}
