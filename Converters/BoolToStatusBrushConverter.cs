using System;
using System.Globalization;
using Avalonia;
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
            _ => isLocal ? GetBrush("LocalBrush") : GetBrush("RemoteBrush")
        };
    }

    public object? ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
    {
        throw new NotImplementedException();
    }

    private static SolidColorBrush GetBrush(string key)
    {
        if (Application.Current?.Resources.TryGetResource(key, null, out var resource) == true && resource is SolidColorBrush brush)
            return brush;

        return key switch
        {
            "LocalBrush" => new SolidColorBrush(Color.Parse("#22C55E")),
            "RemoteBrush" => new SolidColorBrush(Color.Parse("#8B5CF6")),
            "FaintForegroundBrush" => new SolidColorBrush(Color.Parse("#6A6A6A")),
            _ => new SolidColorBrush(Colors.Gray)
        };
    }
}
