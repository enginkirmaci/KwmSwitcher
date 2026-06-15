using Avalonia.Controls;
using Avalonia.Input;

namespace KwmSwitcher.Views;

public partial class MainWindow : Window
{
    public MainWindow()
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