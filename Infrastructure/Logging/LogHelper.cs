using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Serilog;

namespace KwmSwitcher.Infrastructure.Logging;

public static class LogHelper
{
    private static string? _logFilePath;

    public static string LogFilePath => _logFilePath ??= Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "KwmSwitcher",
        "kwmswitcher.log");

    public static void Initialize()
    {
        var logDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "KwmSwitcher");

        Directory.CreateDirectory(logDir);

        _logFilePath = Path.Combine(logDir, "kwmswitcher.log");

        Log.Logger = new LoggerConfiguration()
            .MinimumLevel.Debug()
            .WriteTo.File(
                _logFilePath,
                rollingInterval: RollingInterval.Day,
                retainedFileCountLimit: 7,
                // Keep the buffer short so a hard crash loses at most ~1s of
                // log lines instead of up to 5s.
                flushToDiskInterval: TimeSpan.FromSeconds(1),
                outputTemplate: "{Timestamp:yyyy-MM-dd HH:mm:ss.fff} [{Level:u3}] {Message:lj}{NewLine}{Exception}")
            .CreateLogger();

        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            try
            {
                if (e.ExceptionObject is Exception ex)
                    Log.Fatal(ex, "Unhandled AppDomain exception");
                else
                    Log.Fatal("Unhandled AppDomain exception: {Message}", e.ExceptionObject);

                // Mark the process exit as a crash so an external supervisor
                // (see AppSupervisor) treats this as a relaunchable failure.
                // Native-signal crashes are already non-zero by runtime
                // behavior; this makes the managed path deterministic.
                Environment.ExitCode = 1;
            }
            catch { }
            finally
            {
                Log.CloseAndFlush();
            }
        };

        TaskScheduler.UnobservedTaskException += (_, e) =>
        {
            // Log the error but DO NOT CloseAndFlush here: disposing the shared
            // logger on a single unobserved task error would make every later
            // Log.* call throw ObjectDisposedException, silently swallowing the
            // real crash. Flush only on true process termination.
            try
            {
                Log.Error(e.Exception, "Unobserved task exception");
            }
            catch { }
            e.SetObserved();
        };

        if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
        {
            InstallSignalHandlers();
        }
    }

    public static void Flush()
    {
        try { Log.CloseAndFlush(); }
        catch { }
    }

    private static void InstallSignalHandlers()
    {
        try
        {
            SetupPosixSignalHandler();
        }
        catch { }
    }

    private static void SetupPosixSignalHandler()
    {
        var crashLogPath = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "KwmSwitcher", "crash.log");

        void HandleSignal(string signal)
        {
            try
            {
                var msg = $"[{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff}] Process killed by signal: {signal}";
                File.AppendAllText(crashLogPath, msg + Environment.NewLine);
                Log.Fatal("Process killed by signal: {Signal}", signal);
            }
            catch
            {
                try
                {
                    File.AppendAllText(crashLogPath,
                        $"[{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff}] Process killed by signal: {signal} (flush failed)" +
                        Environment.NewLine);
                }
                catch { }
            }
            finally
            {
                Log.CloseAndFlush();
            }
        }

        // Register the signals a tray app actually receives on logout/shutdown
        // (AppDomain.ProcessExit does NOT fire for these). Without this, the
        // process simply vanishes with no log trail.
        foreach (var signal in new[] { PosixSignal.SIGTERM, PosixSignal.SIGINT, PosixSignal.SIGHUP })
        {
            try
            {
                PosixSignalRegistration.Create(signal, ctx =>
                {
                    HandleSignal(ctx.Signal.ToString());
                    ctx.Cancel = false; // let the default termination proceed
                });
            }
            catch { }
        }

        // NOTE: fatal signals (SIGSEGV/SIGABRT/SIGBUS/SIGFPE/SIGILL) are NOT
        // in the PosixSignal enum — the .NET runtime owns them and they can't
        // be safely intercepted from managed code. Instead we redirect fd 2
        // (stderr) to a file below, so the runtime's own native-crash dump
        // ("Fatal error. Internal CLR error." + stack) is captured rather than
        // lost to the void of a detached tray app.
        RedirectStderrToCrashLog();

        AppDomain.CurrentDomain.ProcessExit += (_, _) =>
        {
            try
            {
                Log.Information("Application process exiting normally");
                Log.CloseAndFlush();
            }
            catch { }
        };
    }

    /// <summary>
    /// Redirects the OS-level stderr (fd 2) to <c>stderr.log</c> next to the
    /// other logs. A detached tray app has no terminal, so without this the
    /// .NET runtime's native-crash dump ("Fatal error. Internal CLR error."
    /// plus a stack trace, written on SIGSEGV/SIGABRT/...) and any
    /// <c>Console.Error</c> output vanishes. Reopening the fd in append mode
    /// at the libc level (not just <c>Console.SetError</c>) is what captures
    /// the runtime's own unmanaged writes.
    /// </summary>
    private static void RedirectStderrToCrashLog()
    {
        try
        {
            var stderrLogPath = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                "KwmSwitcher", "stderr.log");

            // O_WRONLY|O_CREAT|O_APPEND = 0x1 | 0x40 | 0x400 on Linux.
            const int O_WRONLY = 0x1;
            const int O_CREAT  = 0x40;
            const int O_APPEND = 0x400;
            var fd = open(stderrLogPath, O_WRONLY | O_CREAT | O_APPEND, 0x1B6 /* 0644 */);
            if (fd >= 0)
            {
                dup2(fd, 2);   // point fd 2 (stderr) at our file
                close(fd);
            }
        }
        catch { }
    }

    [DllImport("libc", SetLastError = true)]
    private static extern int open(string path, int flags, int mode);

    [DllImport("libc", SetLastError = true)]
    private static extern int dup2(int oldfd, int newfd);

    [DllImport("libc", SetLastError = true)]
    private static extern int close(int fd);
}