using Avalonia;
using System;
using System.IO;
using System.Threading;
using KwmSwitcher.Infrastructure.Logging;
using KwmSwitcher.Infrastructure.Supervision;
using Serilog;

namespace KwmSwitcher;

sealed class Program
{
    private const string SuperviseFlag = "--supervise";
    private const string CrashTestFlag = "--crash-test";

    private static readonly string CrashLogPath = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "KwmSwitcher", "crash.log");

    [STAThread]
    public static int Main(string[] args)
    {
        // Supervisor mode: spawn the app as a child and relaunch on crash,
        // including native faults that managed code can't catch. Dispatch
        // before any logging/Avalonia setup so the supervisor stays lightweight.
        if (ContainsSuperviseFlag(args))
            return AppSupervisor.Run(args);

        LogHelper.Initialize();

        // Diagnostic hook: intentionally crash the process so the supervisor's
        // relaunch behavior can be exercised on demand. Two modes:
        //   --crash-test native    raise SIGSEGV (the case managed code can't catch)
        //   --crash-test managed   unhandled exception on a background thread
        // Defaults to "native" when no mode argument is supplied.
        if (TryGetCrashTestMode(args, out var mode))
        {
            TriggerCrash(mode);
            return 1; // only reached if the crash path failed for some reason
        }

        try
        {
            Log.Debug("Starting KwmSwitcher application");

            return BuildAvaloniaApp()
                .StartWithClassicDesktopLifetime(args);
        }
        catch (Exception ex)
        {
            Log.Fatal(ex, "Application terminated unexpectedly");
            WriteCrashLog($"Application terminated unexpectedly: {ex}");
            return 1; // non-zero so a supervisor (if any) treats this as a crash
        }
        finally
        {
            LogHelper.Flush();
        }
    }

    private static bool ContainsSuperviseFlag(string[] args)
    {
        if (args is null)
            return false;
        foreach (var arg in args)
            if (string.Equals(arg, SuperviseFlag, StringComparison.Ordinal))
                return true;
        return false;
    }

    /// <summary>
    /// Parses the optional value of <c>--crash-test</c>. Returns <c>true</c> if
    /// the flag is present. <paramref name="mode"/> is set to the following
    /// token (lower-cased) when provided, otherwise defaults to "native".
    /// </summary>
    private static bool TryGetCrashTestMode(string[] args, out string mode)
    {
        mode = "native";
        if (args is null)
            return false;

        for (int i = 0; i < args.Length; i++)
        {
            if (string.Equals(args[i], CrashTestFlag, StringComparison.OrdinalIgnoreCase))
            {
                if (i + 1 < args.Length && !args[i + 1].StartsWith('-'))
                    mode = args[i + 1].ToLowerInvariant();
                return true;
            }
        }
        return false;
    }

    /// <summary>
    /// Deliberately crashes the process in the requested way for testing the
    /// supervisor's relaunch behavior.
    /// </summary>
    private static void TriggerCrash(string mode)
    {
        Log.Warning("Crash-test hook firing in '{Mode}' mode", mode);

        // Give the log a moment to flush before the fault.
        Thread.Sleep(200);

        if (string.Equals(mode, "managed", StringComparison.OrdinalIgnoreCase))
        {
            // Unhandled exception on a background thread. The runtime reports
            // it via AppDomain.UnhandledException (which sets ExitCode = 1) and
            // then tears the process down (SIGABRT, exit 134). Either way the
            // exit is non-zero, which is all the supervisor keys on.
            new Thread(() => throw new InvalidOperationException(
                "crash-test: deliberate unhandled exception"))
            { IsBackground = true }.Start();

            // Block so the fault on the background thread actually propagates
            // and tears down the process rather than Main returning normally.
            Thread.Sleep(Timeout.Infinite);
        }
        else
        {
            // Simulate an unrecoverable native-style fault. We use FailFast
            // rather than raise(SIGSEGV/SIGABRT) because .NET 10 intercepts
            // raise()d signals and swallows them (exit 0), so they do NOT
            // actually crash the process. A genuine native fault from inside
            // P/Invoked native code (the real ddcutil/i2c case) cannot be
            // safely reproduced from a managed test hook. FailFast is the
            // closest cross-platform equivalent: it terminates immediately,
            // bypasses exception handlers, and exits non-zero (134 on Linux)
            // — exactly the kind of crash only an external supervisor can
            // recover from.
            Environment.FailFast("crash-test: deliberate unrecoverable failure");
        }
    }

    public static AppBuilder BuildAvaloniaApp()
        => AppBuilder.Configure<App>()
            .UsePlatformDetect()
#if DEBUG
            .WithDeveloperTools()
#endif
            .WithInterFont()
            .LogToTrace();

    private static void WriteCrashLog(string message)
    {
        try
        {
            var crashDir = Path.GetDirectoryName(CrashLogPath);
            if (!string.IsNullOrEmpty(crashDir))
                Directory.CreateDirectory(crashDir);

            File.AppendAllText(CrashLogPath,
                $"[{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff}] {message}{Environment.NewLine}");
        }
        catch { }
    }
}
