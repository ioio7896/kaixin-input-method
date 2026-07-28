#region License Information (GPL v3)

/*
    ShareX - A program that allows you to take screenshots and share any file type
    Copyright (c) 2007-2026 ShareX Team

    This program is free software; you can redistribute it and/or
    modify it under the terms of the GNU General Public License
    as published by the Free Software Foundation; either version 2
    of the License, or (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program; if not, write to the Free Software
    Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA  02110-1301, USA.

    Optionally you can also view the license at <http://www.gnu.org/licenses/>.
*/

#endregion License Information (GPL v3)

using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.Media.Imaging;

namespace ShareX.ImageEditor.Presentation.Controls;

public class ImageComparisonSlider : Control
{
    public static readonly StyledProperty<Bitmap?> LeftImageProperty =
        AvaloniaProperty.Register<ImageComparisonSlider, Bitmap?>(nameof(LeftImage));

    public static readonly StyledProperty<Bitmap?> RightImageProperty =
        AvaloniaProperty.Register<ImageComparisonSlider, Bitmap?>(nameof(RightImage));

    public static readonly StyledProperty<double> SliderPositionProperty =
        AvaloniaProperty.Register<ImageComparisonSlider, double>(nameof(SliderPosition), 0.5);

    private const double CheckerSize = 12d;
    private static readonly IBrush CheckerLightBrush = new SolidColorBrush(Color.FromRgb(235, 235, 235));
    private static readonly IBrush CheckerDarkBrush = new SolidColorBrush(Color.FromRgb(205, 205, 205));
    private static readonly IBrush SliderBrush = new SolidColorBrush(Color.FromRgb(255, 255, 255));
    private static readonly IPen SliderPen = new Pen(SliderBrush, 2);
    private static readonly IBrush HandleBrush = new SolidColorBrush(Color.FromArgb(230, 35, 35, 35));
    private static readonly IPen HandlePen = new Pen(SliderBrush, 2);
    private readonly Cursor _sliderCursor = new(StandardCursorType.SizeWestEast);
    private bool _isDragging;

    public ImageComparisonSlider()
    {
        ClipToBounds = true;
    }

    public Bitmap? LeftImage
    {
        get => GetValue(LeftImageProperty);
        set => SetValue(LeftImageProperty, value);
    }

    public Bitmap? RightImage
    {
        get => GetValue(RightImageProperty);
        set => SetValue(RightImageProperty, value);
    }

    public double SliderPosition
    {
        get => GetValue(SliderPositionProperty);
        set => SetValue(SliderPositionProperty, Math.Clamp(value, 0d, 1d));
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);

        if (change.Property == LeftImageProperty ||
            change.Property == RightImageProperty ||
            change.Property == SliderPositionProperty)
        {
            Cursor = LeftImage != null && RightImage != null ? _sliderCursor : null;
            InvalidateVisual();
        }
    }

    public override void Render(DrawingContext context)
    {
        base.Render(context);

        Rect bounds = new(Bounds.Size);

        Bitmap? leftImage = LeftImage;
        Bitmap? rightImage = RightImage;

        if (leftImage == null && rightImage == null)
        {
            return;
        }

        Rect imageBounds = GetImageBounds(bounds, leftImage, rightImage);
        DrawTransparencyBackground(context, imageBounds);

        if (leftImage == null || rightImage == null)
        {
            DrawBitmap(context, leftImage ?? rightImage!, imageBounds);
            return;
        }

        DrawBitmap(context, rightImage, imageBounds);

        double sliderX = GetSliderX(imageBounds);
        using (context.PushClip(new Rect(imageBounds.Left, imageBounds.Top, Math.Max(0, sliderX - imageBounds.Left), imageBounds.Height)))
        {
            DrawTransparencyBackground(context, imageBounds);
            DrawBitmap(context, leftImage, imageBounds);
        }

        DrawSlider(context, imageBounds);
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        base.OnPointerPressed(e);

        if (LeftImage != null && RightImage != null && e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            _isDragging = true;
            e.Pointer.Capture(this);
            UpdateSliderPosition(e.GetPosition(this));
            e.Handled = true;
        }
    }

    protected override void OnPointerMoved(PointerEventArgs e)
    {
        base.OnPointerMoved(e);

        if (_isDragging)
        {
            UpdateSliderPosition(e.GetPosition(this));
            e.Handled = true;
        }
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        base.OnPointerReleased(e);

        if (_isDragging)
        {
            _isDragging = false;
            e.Pointer.Capture(null);
            UpdateSliderPosition(e.GetPosition(this));
            e.Handled = true;
        }
    }

    private static void DrawBitmap(DrawingContext context, Bitmap bitmap, Rect destination)
    {
        PixelSize pixelSize = bitmap.PixelSize;
        Rect source = new(0, 0, pixelSize.Width, pixelSize.Height);
        context.DrawImage(bitmap, source, destination);
    }

    private static void DrawTransparencyBackground(DrawingContext context, Rect bounds)
    {
        context.FillRectangle(CheckerLightBrush, bounds);

        int columns = (int)Math.Ceiling(bounds.Width / CheckerSize);
        int rows = (int)Math.Ceiling(bounds.Height / CheckerSize);

        for (int y = 0; y < rows; y++)
        {
            for (int x = 0; x < columns; x++)
            {
                if ((x + y) % 2 == 0)
                {
                    continue;
                }

                Rect cell = new(
                    bounds.Left + x * CheckerSize,
                    bounds.Top + y * CheckerSize,
                    Math.Min(CheckerSize, bounds.Right - (bounds.Left + x * CheckerSize)),
                    Math.Min(CheckerSize, bounds.Bottom - (bounds.Top + y * CheckerSize)));

                context.FillRectangle(CheckerDarkBrush, cell);
            }
        }
    }

    private static Rect GetImageBounds(Rect bounds, Bitmap? leftImage, Bitmap? rightImage)
    {
        double imageWidth = Math.Max(leftImage?.PixelSize.Width ?? 0, rightImage?.PixelSize.Width ?? 0);
        double imageHeight = Math.Max(leftImage?.PixelSize.Height ?? 0, rightImage?.PixelSize.Height ?? 0);

        if (imageWidth <= 0 || imageHeight <= 0 || bounds.Width <= 0 || bounds.Height <= 0)
        {
            return bounds;
        }

        double scale = Math.Min(bounds.Width / imageWidth, bounds.Height / imageHeight);
        double width = imageWidth * scale;
        double height = imageHeight * scale;
        double left = Math.Round(bounds.Left + (bounds.Width - width) / 2d);
        double top = Math.Round(bounds.Top + (bounds.Height - height) / 2d);
        double right = Math.Round(bounds.Left + (bounds.Width + width) / 2d);
        double bottom = Math.Round(bounds.Top + (bounds.Height + height) / 2d);

        return new Rect(left, top, Math.Max(1d, right - left), Math.Max(1d, bottom - top));
    }

    private void DrawSlider(DrawingContext context, Rect imageBounds)
    {
        double sliderX = GetSliderX(imageBounds);
        Point top = new(sliderX, imageBounds.Top);
        Point bottom = new(sliderX, imageBounds.Bottom);
        context.DrawLine(SliderPen, top, bottom);

        const double handleWidth = 34;
        const double handleHeight = 52;
        Rect handleBounds = new(
            sliderX - handleWidth / 2d,
            imageBounds.Top + imageBounds.Height / 2d - handleHeight / 2d,
            handleWidth,
            handleHeight);

        RoundedRect roundedHandle = new(handleBounds, 16);
        context.DrawRectangle(HandleBrush, HandlePen, roundedHandle);
        context.DrawLine(SliderPen, new Point(sliderX - 5, handleBounds.Top + 17), new Point(sliderX - 5, handleBounds.Bottom - 17));
        context.DrawLine(SliderPen, new Point(sliderX + 5, handleBounds.Top + 17), new Point(sliderX + 5, handleBounds.Bottom - 17));
    }

    private void UpdateSliderPosition(Point pointerPosition)
    {
        Rect imageBounds = GetImageBounds(new Rect(Bounds.Size), LeftImage, RightImage);
        SliderPosition = imageBounds.Width <= 0
            ? 0.5
            : Math.Clamp((pointerPosition.X - imageBounds.Left) / imageBounds.Width, 0d, 1d);
    }

    private double GetSliderX(Rect imageBounds)
    {
        return Math.Round(imageBounds.Left + imageBounds.Width * SliderPosition);
    }
}
