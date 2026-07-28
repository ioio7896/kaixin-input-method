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

using Avalonia.Data.Converters;
using ShareX.ImageEditor.Core.Annotations;
using ShareX.ImageEditor.Presentation.Localization;
using System.Globalization;

namespace ShareX.ImageEditor.Presentation.Converters
{
    public class CursorTypeDisplayNameConverter : IValueConverter
    {
        public object Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
        {
            CursorType cursorType = value is CursorType typedCursor ? typedCursor : CursorType.Default;

            return cursorType switch
            {
                CursorType.AppStarting => EditorLocalizer.Tr("App starting", "应用启动"),
                CursorType.Arrow => EditorLocalizer.Tr("Arrow", "箭头"),
                CursorType.Cross => EditorLocalizer.Tr("Cross", "十字"),
                CursorType.Default => EditorLocalizer.Tr("Default", "默认"),
                CursorType.Hand => EditorLocalizer.Tr("Hand", "手形"),
                CursorType.Help => EditorLocalizer.Tr("Help", "帮助"),
                CursorType.HSplit => EditorLocalizer.Tr("H split", "水平拆分"),
                CursorType.IBeam => EditorLocalizer.Tr("I-beam", "文本光标"),
                CursorType.No => EditorLocalizer.Tr("No", "禁止"),
                CursorType.NoMove2D => EditorLocalizer.Tr("No move 2D", "禁止二维移动"),
                CursorType.NoMoveHoriz => EditorLocalizer.Tr("No move horiz", "禁止水平移动"),
                CursorType.NoMoveVert => EditorLocalizer.Tr("No move vert", "禁止垂直移动"),
                CursorType.PanEast => EditorLocalizer.Tr("Pan east", "向东平移"),
                CursorType.PanNE => EditorLocalizer.Tr("Pan NE", "向东北平移"),
                CursorType.PanNorth => EditorLocalizer.Tr("Pan north", "向北平移"),
                CursorType.PanNW => EditorLocalizer.Tr("Pan NW", "向西北平移"),
                CursorType.PanSE => EditorLocalizer.Tr("Pan SE", "向东南平移"),
                CursorType.PanSouth => EditorLocalizer.Tr("Pan south", "向南平移"),
                CursorType.PanSW => EditorLocalizer.Tr("Pan SW", "向西南平移"),
                CursorType.PanWest => EditorLocalizer.Tr("Pan west", "向西平移"),
                CursorType.SizeAll => EditorLocalizer.Tr("Size all", "全向缩放"),
                CursorType.SizeNESW => EditorLocalizer.Tr("Size NESW", "对角缩放 NESW"),
                CursorType.SizeNS => EditorLocalizer.Tr("Size NS", "垂直缩放"),
                CursorType.SizeNWSE => EditorLocalizer.Tr("Size NWSE", "对角缩放 NWSE"),
                CursorType.SizeWE => EditorLocalizer.Tr("Size WE", "水平缩放"),
                CursorType.UpArrow => EditorLocalizer.Tr("Up arrow", "上箭头"),
                CursorType.VSplit => EditorLocalizer.Tr("V split", "垂直拆分"),
                CursorType.WaitCursor => EditorLocalizer.Tr("Wait cursor", "等待光标"),
                _ => cursorType.ToString()
            };
        }

        public object ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
        {
            return CursorType.Default;
        }
    }
}
