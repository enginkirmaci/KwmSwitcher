using System;
using System.Globalization;
using Avalonia;
using Avalonia.Data;
using Avalonia.Data.Converters;
using Avalonia.Media;

namespace KwmSwitcher.Converters;

public class BoolToStatusBrushConverter : IValueConverter
{
    public object? Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
    {
        bool isLocal = value is true;
        var side = parameter as string;

        return side switch
        {
            "Local" => isLocal ? GetBrush("LocalBrush") : GetBrush("FaintForegroundBrush"),
            "Remote" => !isLocal ? GetBrush("RemoteBrush") : GetBrush("FaintForegroundBrush"),
            "Pip" => isLocal ? GetBrush("AccentBrush") : GetBrush("BodyForegroundBrush"),
            _ => isLocal ? GetBrush("LocalBrush") : GetBrush("RemoteBrush")
        };
    }

    // One-way converter: no meaningful back-conversion.
    public object? ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
        => BindingOperations.DoNothing;

    private static SolidColorBrush GetBrush(string key)
    {
        if (Application.Current?.Resources.TryGetResource(key, null, out var resource) == true && resource is SolidColorBrush brush)
            return brush;

        return key switch
        {
            "LocalBrush" => new SolidColorBrush(Color.Parse("#22C55E")),
            "RemoteBrush" => new SolidColorBrush(Color.Parse("#8B5CF6")),
            "AccentBrush" => new SolidColorBrush(Color.Parse("#3B82F6")),
            "FaintForegroundBrush" => new SolidColorBrush(Color.Parse("#6A6A6A")),
            "BodyForegroundBrush" => new SolidColorBrush(Color.Parse("#DCDCDC")),
            _ => new SolidColorBrush(Colors.Gray)
        };
    }
}
