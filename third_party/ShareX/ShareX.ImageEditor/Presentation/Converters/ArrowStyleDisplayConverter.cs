using Avalonia.Data.Converters;
using ShareX.ImageEditor.Core.Annotations;
using ShareX.ImageEditor.Presentation.Localization;
using System.Globalization;

namespace ShareX.ImageEditor.Presentation.Converters
{
    public class ArrowStyleDisplayConverter : IValueConverter
    {
        public object? Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
        {
            ArrowStyle arrowStyle = value is ArrowStyle typedValue ? typedValue : ArrowStyle.Classic;
            return EditorLocalizer.TranslateArrowStyle(arrowStyle.ToString());
        }

        public object? ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
        {
            return ArrowStyle.Classic;
        }
    }
}
