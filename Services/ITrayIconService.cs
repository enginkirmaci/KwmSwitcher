using System;
using System.Threading.Tasks;

namespace KwmSwitcher.Services;

/// <summary>
/// Owns the system tray icon and its context menu.
/// Handlers for menu actions are supplied by the host so this service stays
/// free of business logic — it only builds the menu and routes clicks.
/// </summary>
public interface ITrayIconService : IDisposable
{
    void Initialize(
        Action showMainWindow,
        Action openSettings,
        Func<Task> switchToLocal,
        Func<Task> switchToRemote,
        Func<Task> togglePip,
        Action quit);
}
