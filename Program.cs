using System;
using System.Runtime.InteropServices;
using Avalonia;

namespace KwmSwitcher;

sealed class Program
{
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr GetModuleHandle(string lpModuleName);

    [DllImport("kernel32.dll")]
    private static extern IntPtr GetProcAddress(IntPtr hModule, string lpProcName);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate bool SetProcessGPUPreferenceDelegate(int gpuPreference);

    [STAThread]
    public static void Main(string[] args)
    {
        ForceDiscreteGpu();
        BuildAvaloniaApp().StartWithClassicDesktopLifetime(args);
    }

    private static void ForceDiscreteGpu()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            return;

        var kernel32 = GetModuleHandle("kernel32.dll");
        if (kernel32 == IntPtr.Zero)
            return;

        var procAddress = GetProcAddress(kernel32, "SetProcessGPUPreference");
        if (procAddress == IntPtr.Zero)
            return;

        try
        {
            var setGpuPref = Marshal.GetDelegateForFunctionPointer<SetProcessGPUPreferenceDelegate>(procAddress);
            setGpuPref(2);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Failed to set GPU preference: {ex.Message}");
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