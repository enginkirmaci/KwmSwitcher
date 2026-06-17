using Avalonia;
using System;
using System.IO;
using System.Runtime.InteropServices;
using KwmSwitcher.Infrastructure.Logging;
using Serilog;

namespace KwmSwitcher;

sealed class Program
{
    private static readonly string CrashLogPath = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "KwmSwitcher", "crash.log");

    [STAThread]
    public static void Main(string[] args)
    {
        LogHelper.Initialize();
        try
        {
            Log.Debug("Starting KwmSwitcher application");

            if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
            {
                InstallLinuxSignalHandlers();
            }

            BuildAvaloniaApp()
                .StartWithClassicDesktopLifetime(args);
        }
        catch (Exception ex)
        {
            Log.Fatal(ex, "Application terminated unexpectedly");
            WriteCrashLog($"Application terminated unexpectedly: {ex}");
        }
        finally
        {
            LogHelper.Flush();
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

    private static void InstallLinuxSignalHandlers()
    {
        try
        {
            AppDomain.CurrentDomain.ProcessExit += (_, _) =>
            {
                try
                {
                    Log.Information("Application process exiting normally");
                }
                catch { }
            };
        }
        catch { }
    }
}