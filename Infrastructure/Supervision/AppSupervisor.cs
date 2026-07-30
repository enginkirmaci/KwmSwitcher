using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using Serilog;

namespace KwmSwitcher.Infrastructure.Supervision;

/// <summary>
/// An external supervisor that spawns the application as a child process and
/// relaunches it after a crash.
/// </summary>
/// <remarks>
/// <para>
/// This exists because the crashes we care about most are <b>native</b> faults
/// (SIGSEGV/SIGABRT/SIGBUS) raised by the .NET runtime or native dependencies
/// such as <c>ddcutil</c>/i2c. The runtime owns those signals and they cannot
/// be safely intercepted from managed code, so an in-process "respawn myself
/// before dying" approach is helpless against them. A <i>separate</i> process
/// that merely observes the child's exit code has no such limitation: any
/// non-zero exit (managed exception, native crash, or signal kill) is treated
/// as a crash and triggers a relaunch.
/// </para>
/// <para>
/// <b>Exit-code contract:</b>
/// <list type="bullet">
/// <item><c>0</c> = clean exit (user clicked Quit in the tray, or the
/// supervisor itself is shutting down on logout). The supervisor does
/// <b>not</b> relaunch.</item>
/// <item><c>non-zero</c> = crash. The supervisor relaunches after a backoff,
/// unless a crash loop is detected.</item>
/// </list>
/// </para>
/// <para>
/// This supervisor does not load Avalonia and does not touch the application's
/// rolling log; it writes its own <c>supervisor.log</c> to avoid concurrent
/// file-handle contention with the supervised child.
/// </para>
/// </summary>
public static class AppSupervisor
{
    private const string SuperviseFlag = "--supervise";

    // Backoff tuning for the relaunch delay after a crash.
    private static readonly TimeSpan InitialBackoff = TimeSpan.FromSeconds(1);
    private static readonly TimeSpan MaxBackoff = TimeSpan.FromSeconds(30);

    // Crash-loop detection. If the child dies this many times in a row without
    // staying up long enough to be considered "healthy", we give up to avoid
    // hammering a broken environment. The count is consecutive (not a time
    // window) because the exponential backoff widens the gaps between
    // relaunches — a fixed window would let crashes age out before tripping.
    private const int CrashLoopThreshold = 5;

    // Uptime after which a crash is treated as a fresh incident rather than
    // part of a startup crash loop. Keeps a normally-running app from being
    // killed by the loop guard after one fluke crash.
    private static readonly TimeSpan HealthyUptime = TimeSpan.FromSeconds(60);

    private static readonly string SupervisorLogPath = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "KwmSwitcher", "supervisor.log");

    private static readonly ManualResetEventSlim StopEvent = new(false);

    /// <summary>
    /// Strips the <c>--supervise</c> flag from the argument list so it is not
    /// forwarded to the supervised child (otherwise the child would itself
    /// try to supervise, recursing indefinitely).
    /// </summary>
    public static IReadOnlyList<string> StripSuperviseFlag(string[] args)
    {
        if (args is null || args.Length == 0)
            return Array.Empty<string>();

        var stripped = new List<string>(args.Length);
        foreach (var arg in args)
        {
            if (!string.Equals(arg, SuperviseFlag, StringComparison.Ordinal))
                stripped.Add(arg);
        }
        return stripped;
    }

    /// <summary>
    /// Runs the supervision loop. Returns the process exit code that
    /// <see cref="Program.Main"/> should propagate.
    /// </summary>
    /// <param name="args">
    /// The original command-line arguments <i>including</i> the
    /// <c>--supervise</c> flag; it is stripped before being passed to the
    /// child.
    /// </param>
    public static int Run(string[] args)
    {
        InitializeLogger();
        var childArgs = StripSuperviseFlag(args);

        Log.Information("KwmSwitcher supervisor starting; child args: {Args}",
            childArgs.Count == 0 ? "(none)" : string.Join(' ', childArgs));

        InstallSignalHandlers();

        int consecutiveCrashes = 0;
        var currentBackoff = InitialBackoff;

        try
        {
            while (!StopEvent.IsSet)
            {
                // Test seam: spawn an alternate child when the override env var
                // is set, so the exit-0/crash contracts can be exercised against
                // this real supervisor code without the GUI. Inert in production.
                var childPath = Environment.GetEnvironmentVariable("KWMSWITCHER_SUPERVISE_CHILD");
                if (string.IsNullOrEmpty(childPath))
                    childPath = Environment.ProcessPath;
                if (string.IsNullOrEmpty(childPath) || !File.Exists(childPath))
                {
                    Log.Fatal("Cannot determine executable path to relaunch ({Path}); giving up.", childPath ?? "(null)");
                    return 1;
                }

                var start = DateTime.UtcNow;
                int exitCode = SpawnAndWait(childPath, childArgs);
                var uptime = DateTime.UtcNow - start;

                // Supervisor is shutting down (logout/shutdown). Don't relaunch.
                if (StopEvent.IsSet)
                {
                    Log.Information("Supervisor stopping; child exited with code {ExitCode}.", exitCode);
                    break;
                }

                // Exit code 0 == clean exit (user Quit). Stop without relaunch.
                if (exitCode == 0)
                {
                    Log.Information("Child exited cleanly (code 0). Supervisor will not relaunch.");
                    break;
                }

                Log.Warning("Child crashed with exit code {ExitCode} after {Uptime}.", exitCode, uptime);

                // If it ran long enough, treat this as a fresh incident and
                // reset the crash-loop state and backoff.
                if (uptime >= HealthyUptime)
                {
                    consecutiveCrashes = 0;
                    currentBackoff = InitialBackoff;
                    Log.Information("Uptime >= {Healthy} so treating this as a fresh incident; crash counters reset.",
                        HealthyUptime);
                }

                consecutiveCrashes++;

                // Crash-loop detection. We track CONSECUTIVE crashes rather
                // than crashes within a time window: the exponential backoff
                // grows the gaps between relaunches, so a fixed time window
                // would let old crashes age out before the threshold is hit,
                // making the guard effectively unreachable. The count resets
                // on any healthy run (above), so this only fires when the app
                // truly can't stay up.
                if (consecutiveCrashes > CrashLoopThreshold)
                {
                    var msg = $"KwmSwitcher crashed {consecutiveCrashes} times in a row without staying up. " +
                              "Supervisor giving up to avoid a crash loop. Please restart manually.";
                    Log.Fatal(msg);
                    NotifyUser("KwmSwitcher crash loop", msg);
                    return 1;
                }

                Log.Information("Relaunching in {Backoff} (consecutive crash #{Count}).",
                    currentBackoff, consecutiveCrashes);

                if (!WaitWithStop(currentBackoff))
                {
                    // Interrupted by a stop signal during backoff: don't relaunch.
                    Log.Information("Backoff interrupted by stop signal; not relaunching.");
                    break;
                }

                // Exponential backoff, capped.
                currentBackoff = TimeSpan.FromTicks(Math.Min(currentBackoff.Ticks * 2, MaxBackoff.Ticks));
            }
        }
        catch (Exception ex)
        {
            Log.Fatal(ex, "Supervisor loop terminated unexpectedly.");
            return 1;
        }
        finally
        {
            Log.Information("KwmSwitcher supervisor exiting.");
            Log.CloseAndFlush();
        }

        return 0;
    }

    /// <summary>
    /// Spawns the supervised child and blocks until it exits. The wait is
    /// interruptible by <see cref="StopEvent"/> (set on a stop signal).
    /// </summary>
    private static int SpawnAndWait(string childPath, IReadOnlyList<string> childArgs)
    {
        var psi = new ProcessStartInfo
        {
            FileName = childPath,
            UseShellExecute = false,
        };
        for (int i = 0; i < childArgs.Count; i++)
            psi.ArgumentList.Add(childArgs[i]);

        var process = Process.Start(psi);
        if (process is null)
        {
            Log.Error("Process.Start returned null for {Path}; treating as crash.", childPath);
            return -1;
        }

        // Poll so a stop signal can wake us out of WaitForExit promptly.
        while (!process.HasExited)
        {
            if (process.WaitForExit(250))
                break;
            if (StopEvent.IsSet)
            {
                // The session is ending. Propagate the stop signal to the
                // child so it can flush its own logs, then reap it.
                try { process.Kill(entireProcessTree: true); } catch { }
                break;
            }
        }

        try
        {
            process.WaitForExit();
        }
        catch { }

        return process.ExitCode;
    }

    /// <summary>
    /// Waits for the given delay, but returns <c>false</c> early if a stop
    /// signal is received in the meantime.
    /// </summary>
    private static bool WaitWithStop(TimeSpan delay)
    {
        try
        {
            if (StopEvent.Wait(delay))
                return false; // signaled => stop requested
        }
        catch (ArgumentOutOfRangeException)
        {
            // Total wait was zero or negative; nothing to wait for.
        }
        return !StopEvent.IsSet;
    }

    /// <summary>
    /// Installs POSIX signal handlers for the signals a tray/supervisor
    /// process receives on logout/shutdown, so it can exit <b>without</b>
    /// relaunching the child (the session is ending).
    /// </summary>
    private static void InstallSignalHandlers()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
            return;

        foreach (var signal in new[] { PosixSignal.SIGTERM, PosixSignal.SIGINT, PosixSignal.SIGHUP })
        {
            try
            {
                PosixSignalRegistration.Create(signal, ctx =>
                {
                    Log.Information("Supervisor received {Signal}; shutting down without relaunch.", ctx.Signal);
                    ctx.Cancel = true; // we handle termination ourselves
                    try { StopEvent.Set(); } catch { }
                });
            }
            catch (Exception ex)
            {
                Log.Warning(ex, "Failed to register handler for {Signal}.", signal);
            }
        }
    }

    /// <summary>
    /// Best-effort desktop notification via <c>notify-send</c> so a user who
    /// can't see the tray icon is aware the supervisor gave up. Failure is
    /// swallowed — this is purely cosmetic.
    /// </summary>
    private static void NotifyUser(string title, string body)
    {
        try
        {
            var psi = new ProcessStartInfo
            {
                FileName = "notify-send",
                UseShellExecute = false,
            };
            psi.ArgumentList.Add(title);
            psi.ArgumentList.Add(body);
            Process.Start(psi);
        }
        catch (Exception ex)
        {
            Log.Debug(ex, "notify-send unavailable; skipping desktop notification.");
        }
    }

    /// <summary>
    /// Configures a Serilog logger dedicated to the supervisor and writing to
    /// <c>supervisor.log</c>, separate from the application's own rolling log
    /// so the two processes never contend for the same file handle.
    /// </summary>
    private static void InitializeLogger()
    {
        var logDir = Path.GetDirectoryName(SupervisorLogPath);
        if (!string.IsNullOrEmpty(logDir))
            Directory.CreateDirectory(logDir);

        Log.Logger = new LoggerConfiguration()
            .MinimumLevel.Information()
            .WriteTo.File(
                SupervisorLogPath,
                rollingInterval: RollingInterval.Day,
                retainedFileCountLimit: 7,
                flushToDiskInterval: TimeSpan.FromSeconds(1),
                outputTemplate: "{Timestamp:yyyy-MM-dd HH:mm:ss.fff} [{Level:u3}] {Message:lj}{NewLine}{Exception}")
            .CreateLogger();
    }
}
