using Avalonia.Data.Converters;
using ShareX.ImageEditor.Presentation.Localization;
using System.Globalization;

namespace ShareX.ImageEditor.Presentation.Converters
{
    public class ThemeDisplayConverter : IValueConverter
    {
        public object? Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
        {
            return value is string theme ? EditorLocalizer.TranslateTheme(theme) : value;
        }

        public object? ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
        {
            if (value is not string theme)
            {
                return "Dark";
            }

            return theme switch
            {
                "浅色" => "Light",
                "深色" => "Dark",
                _ => theme
            };
        }
    }
}
