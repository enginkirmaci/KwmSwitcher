using System;
using System.Globalization;
using Avalonia.Data.Converters;
using Avalonia.Media;

namespace KwmSwitcher.Converters;

public class BoolToStatusBrushConverter : IValueConverter
{
    public object? Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
    {
        if (value is bool isLocal && isLocal)
            return new SolidColorBrush(Color.Parse("#22C55E"));
        return new SolidColorBrush(Color.Parse("#8B5CF6"));
    }

    public object? ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
    {
        throw new NotImplementedException();
    }
}