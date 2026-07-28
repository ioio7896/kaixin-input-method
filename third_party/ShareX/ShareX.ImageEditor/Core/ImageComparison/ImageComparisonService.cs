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

using SkiaSharp;

namespace ShareX.ImageEditor.Core.ImageComparison;

public sealed class ImageComparisonService
{
    public ImageComparisonResult Compare(SKBitmap image1, SKBitmap image2)
    {
        ArgumentNullException.ThrowIfNull(image1);
        ArgumentNullException.ThrowIfNull(image2);

        int width = Math.Max(image1.Width, image2.Width);
        int height = Math.Max(image1.Height, image2.Height);

        if (width <= 0 || height <= 0)
        {
            return new ImageComparisonResult(new SKBitmap(1, 1), 1, 1);
        }

        SKBitmap diffBitmap = new SKBitmap(new SKImageInfo(width, height, SKColorType.Bgra8888, SKAlphaType.Opaque));
        long matchingPixels = 0;
        long totalPixels = (long)width * height;

        for (int y = 0; y < height; y++)
        {
            for (int x = 0; x < width; x++)
            {
                bool hasPixel1 = x < image1.Width && y < image1.Height;
                bool hasPixel2 = x < image2.Width && y < image2.Height;
                bool matches = hasPixel1 && hasPixel2 && image1.GetPixel(x, y) == image2.GetPixel(x, y);

                if (matches)
                {
                    matchingPixels++;
                }

                diffBitmap.SetPixel(x, y, matches ? SKColors.Black : SKColors.White);
            }
        }

        return new ImageComparisonResult(diffBitmap, matchingPixels, totalPixels);
    }
}
