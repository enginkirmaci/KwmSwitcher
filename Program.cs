using Avalonia;
using System;
using Serilog;

namespace KwmSwitcher;

sealed class Program
{
    [STAThread]
    public static void Main(string[] args)
    {
        LogHelper.Initialize();
        try
        {
            Log.Debug("Starting KwmSwitcher application");

            BuildAvaloniaApp()
                .StartWithClassicDesktopLifetime(args);
        }
        catch (Exception ex)
        {
            Log.Fatal(ex, "Application terminated unexpectedly");
        }
        finally
        {
            Log.CloseAndFlush();
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
}