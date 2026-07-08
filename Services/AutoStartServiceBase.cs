using Serilog;

namespace KwmSwitcher.Services;

/// <summary>
/// Shared error-handling shell for platform autostart services.
///
/// Every autostart operation (check / enable / disable) is fallible: the
/// filesystem or registry may be locked, the user may lack permission, etc.
/// Historically each of the six concrete methods wrapped its body in the same
/// <c>try { … } catch (Exception ex) { Log.Error(...); return default; }</c>
/// block. This base centralizes that wrapper so subclasses can focus on the
/// platform-specific mechanic.
/// </summary>
public abstract class AutoStartServiceBase : IAutoStartService
{
    public bool IsEnabled() => Run("check autostart status", () => IsEnabledCore(), defaultValue: false);

    public void Enable() => Run("enable autostart", EnableCore);

    public void Disable() => Run("disable autostart", DisableCore);

    /// <summary>Platform-specific status check. Exceptions are logged and mapped to <c>false</c>.</summary>
    protected abstract bool IsEnabledCore();

    /// <summary>Platform-specific enable action. Exceptions are logged and swallowed.</summary>
    protected abstract void EnableCore();

    /// <summary>Platform-specific disable action. Exceptions are logged and swallowed.</summary>
    protected abstract void DisableCore();

    /// <summary>
    /// Runs <paramref name="action"/>, logging any exception under
    /// <paramref name="label"/> (e.g. "enable autostart"). No default value is
    /// needed for the void overload.
    /// </summary>
    private static void Run(string label, System.Action action)
    {
        try { action(); }
        catch (System.Exception ex) { Log.Error(ex, "Failed to {Label}", label); }
    }

    /// <summary>
    /// Runs <paramref name="func"/>, returning its result or
    /// <paramref name="defaultValue"/> on failure (exception logged under
    /// <paramref name="label"/>).
    /// </summary>
    private static T Run<T>(string label, System.Func<T> func, T defaultValue)
    {
        try { return func(); }
        catch (System.Exception ex)
        {
            Log.Error(ex, "Failed to {Label}", label);
            return defaultValue;
        }
    }
}
