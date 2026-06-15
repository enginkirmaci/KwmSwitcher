using Avalonia.Controls;
using Avalonia.Input;

namespace KwmSwitcher.Views;

public partial class SettingsWindow : Window
{
    public SettingsWindow()
    {
        InitializeComponent();
    }

    private void TitleBar_PointerPressed(object? sender, PointerPressedEventArgs e)
    {
        var point = e.GetCurrentPoint(this);
        if (point.Properties.IsLeftButtonPressed)
        {
            BeginMoveDrag(e);
        }
    }
}