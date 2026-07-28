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

using Avalonia.Media;
using Avalonia.Media.Imaging;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using ShareX.ImageEditor.Core.ImageComparison;
using ShareX.ImageEditor.Presentation.Localization;
using ShareX.ImageEditor.Presentation.Rendering;
using SkiaSharp;
using System.Globalization;

namespace ShareX.ImageEditor.Presentation.ViewModels;

public enum ImageComparerMode
{
    Slider,
    DiffView
}

public sealed partial class ImageComparerViewModel : ViewModelBase, IDisposable
{
    private readonly ImageComparisonService _comparisonService = new();
    private SKBitmap? _image1Bitmap;
    private SKBitmap? _image2Bitmap;
    private ImageComparisonResult? _comparisonResult;

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(HasImage1))]
    private string? _image1Path;

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(HasImage2))]
    private string? _image2Path;

    [ObservableProperty]
    private Bitmap? _image1Preview;

    [ObservableProperty]
    private Bitmap? _image2Preview;

    [ObservableProperty]
    private Bitmap? _diffPreview;

    [ObservableProperty]
    private Bitmap? _displayedLeftImage;

    [ObservableProperty]
    private Bitmap? _displayedRightImage;

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(IsSliderMode))]
    [NotifyPropertyChangedFor(nameof(IsDiffViewMode))]
    [NotifyPropertyChangedFor(nameof(IsSliderComparisonVisible))]
    [NotifyPropertyChangedFor(nameof(IsDiffComparisonVisible))]
    private ImageComparerMode _selectedMode;

    [ObservableProperty]
    private string _statusText = EditorLocalizer.SelectTwoImagesToCompare;

    [ObservableProperty]
    private string _similarityText = EditorLocalizer.SimilarityUnknown;

    [ObservableProperty]
    private IBrush _similarityBrush = Brushes.White;

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(CanShowComparison))]
    [NotifyPropertyChangedFor(nameof(IsSliderComparisonVisible))]
    [NotifyPropertyChangedFor(nameof(IsDiffComparisonVisible))]
    private bool _hasComparison;

    public Func<string, Task<string?>>? SelectImageFileRequested { get; set; }

    public bool HasImage1 => !string.IsNullOrEmpty(Image1Path);

    public bool HasImage2 => !string.IsNullOrEmpty(Image2Path);

    public bool CanShowComparison => HasComparison;

    public bool IsSliderMode
    {
        get => SelectedMode == ImageComparerMode.Slider;
        set
        {
            if (value)
            {
                SelectedMode = ImageComparerMode.Slider;
            }
        }
    }

    public bool IsDiffViewMode
    {
        get => SelectedMode == ImageComparerMode.DiffView;
        set
        {
            if (value)
            {
                SelectedMode = ImageComparerMode.DiffView;
            }
        }
    }

    public bool IsSliderComparisonVisible => CanShowComparison && SelectedMode == ImageComparerMode.Slider;

    public bool IsDiffComparisonVisible => CanShowComparison && SelectedMode == ImageComparerMode.DiffView;

    [RelayCommand]
    private async Task SelectImage1Async()
    {
        await SelectImageAsync(1);
    }

    [RelayCommand]
    private async Task SelectImage2Async()
    {
        await SelectImageAsync(2);
    }

    private async Task SelectImageAsync(int imageNumber)
    {
        if (SelectImageFileRequested == null)
        {
            StatusText = EditorLocalizer.ImagePickerUnavailable;
            return;
        }

        string? filePath = await SelectImageFileRequested(EditorLocalizer.SelectImageTitle(imageNumber));
        if (string.IsNullOrEmpty(filePath))
        {
            return;
        }

        try
        {
            SKBitmap? bitmap = SKBitmap.Decode(filePath);
            if (bitmap == null)
            {
                StatusText = EditorLocalizer.SelectedFileNotImage;
                return;
            }

            SetImage(imageNumber, filePath, bitmap);
            UpdateComparison();
        }
        catch (Exception ex)
        {
            StatusText = EditorLocalizer.FailedToLoadImage(ex.Message);
        }
    }

    private void SetImage(int imageNumber, string filePath, SKBitmap bitmap)
    {
        Bitmap preview = BitmapConversionHelpers.ToAvaloniBitmap(bitmap);

        if (imageNumber == 1)
        {
            _image1Bitmap?.Dispose();
            Image1Preview?.Dispose();
            _image1Bitmap = bitmap;
            Image1Preview = preview;
            Image1Path = filePath;
        }
        else
        {
            _image2Bitmap?.Dispose();
            Image2Preview?.Dispose();
            _image2Bitmap = bitmap;
            Image2Preview = preview;
            Image2Path = filePath;
        }
    }

    private void UpdateComparison()
    {
        HasComparison = false;
        _comparisonResult?.Dispose();
        _comparisonResult = null;
        DiffPreview?.Dispose();
        DiffPreview = null;

        if (_image1Bitmap == null || _image2Bitmap == null)
        {
            StatusText = EditorLocalizer.SelectTwoImagesToCompare;
            SimilarityText = EditorLocalizer.SimilarityUnknown;
            SimilarityBrush = Brushes.White;
            RefreshDisplayedImages();
            return;
        }

        _comparisonResult = _comparisonService.Compare(_image1Bitmap, _image2Bitmap);
        DiffPreview = BitmapConversionHelpers.ToAvaloniBitmap(_comparisonResult.DiffBitmap);

        double similarity = _comparisonResult.SimilarityPercentage;
        SimilarityText = EditorLocalizer.Similarity(similarity);
        SimilarityBrush = Math.Abs(similarity - 100d) < 0.0001
            ? new SolidColorBrush(Color.FromRgb(16, 185, 129))
            : Brushes.White;
        StatusText = EditorLocalizer.MatchingPixels(_comparisonResult.MatchingPixels, _comparisonResult.TotalPixels);
        HasComparison = true;

        RefreshDisplayedImages();
    }

    private void RefreshDisplayedImages()
    {
        DisplayedLeftImage = Image1Preview;
        DisplayedRightImage = Image2Preview;
    }

    public void Dispose()
    {
        _image1Bitmap?.Dispose();
        _image2Bitmap?.Dispose();
        _comparisonResult?.Dispose();
        Image1Preview?.Dispose();
        Image2Preview?.Dispose();
        DiffPreview?.Dispose();
    }
}
