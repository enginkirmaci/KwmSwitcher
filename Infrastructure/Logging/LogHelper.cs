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
        $"kwmswitcher-{DateTime.Now:yyyyMMdd}.log");

    public static void Initialize()
    {
        var logDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "KwmSwitcher");

        Directory.CreateDirectory(logDir);

        _logFilePath = Path.Combine(logDir, $"kwmswitcher-{DateTime.Now:yyyyMMdd}.log");

        Log.Logger = new LoggerConfiguration()
            .MinimumLevel.Debug()
            .WriteTo.File(
                _logFilePath,
                rollingInterval: RollingInterval.Day,
                retainedFileCountLimit: 7,
                flushToDiskInterval: TimeSpan.FromSeconds(5),
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
            }
            catch { }
            finally
            {
                Log.CloseAndFlush();
            }
        };

        TaskScheduler.UnobservedTaskException += (_, e) =>
        {
            try
            {
                Log.Error(e.Exception, "Unobserved task exception");
                Log.CloseAndFlush();
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

        Action<string> writeCrashLog = signal =>
        {
            try
            {
                var msg = $"[{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff}] Process killed by signal: {signal}";
                File.AppendAllText(crashLogPath, msg + Environment.NewLine);
                Log.Fatal("Process killed by signal: {Signal}", signal);
                Log.CloseAndFlush();
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
        };

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
}