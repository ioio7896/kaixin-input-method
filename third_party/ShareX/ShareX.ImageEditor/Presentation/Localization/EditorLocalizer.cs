using System.Globalization;

namespace ShareX.ImageEditor.Presentation.Localization;

public static class EditorLocalizer
{
    private static readonly bool UseSimplifiedChinese =
        CultureInfo.CurrentUICulture.Name.StartsWith("zh-CN", StringComparison.OrdinalIgnoreCase);

    public static string Tr(string english, string simplifiedChinese) =>
        UseSimplifiedChinese ? simplifiedChinese : english;

    public static string Format(string englishTemplate, string chineseTemplate, params object[] args) =>
        string.Format(CultureInfo.CurrentCulture, UseSimplifiedChinese ? chineseTemplate : englishTemplate, args);

    public static string WindowTitleBase => Tr("ShareX - Image Editor", "ShareX - 图片编辑器");
    public static string BackgroundRemoverWindowTitle => Tr("ShareX - Background Remover", "ShareX - 背景移除");
    public static string ImageComparerWindowTitle => Tr("ShareX - Image Comparer", "ShareX - 图片对比");

    public static string StartScreen => Tr("Start Screen", "开始页");
    public static string CreateNewImage => Tr("Create new image...", "新建图片...");
    public static string OpenImageFile => Tr("Open image file...", "打开图片文件...");
    public static string LoadImageFromClipboard => Tr("Load image from clipboard", "从剪贴板载入图片");
    public static string LoadImageFromUrl => Tr("Load image from URL...", "从 URL 载入图片...");
    public static string Load => Tr("Load", "载入");
    public static string Apply => Tr("Apply", "应用");
    public static string Cancel => Tr("Cancel", "取消");
    public static string Exit => Tr("Exit", "退出");
    public static string RecentFiles => Tr("Recent files", "最近文件");
    public static string NoRecentImageFilesYet => Tr("No recent image files yet.", "还没有最近打开的图片文件。");
    public static string InvalidHttpUrl => Tr("Please enter a valid HTTP or HTTPS URL.", "请输入有效的 HTTP 或 HTTPS URL。");

    public static string NewImage => Tr("New Image", "新建图片");
    public static string Width => Tr("Width:", "宽度：");
    public static string Height => Tr("Height:", "高度：");
    public static string Pixels => Tr("pixels", "像素");
    public static string Background => Tr("Background", "背景");
    public static string Transparent => Tr("Transparent", "透明");
    public static string SolidColor => Tr("Solid color", "纯色");

    public static string Yes => Tr("Yes", "是");
    public static string No => Tr("No", "否");
    public static string ExitConfirmation => Tr("Exit Confirmation", "退出确认");
    public static string UnsavedChangesPrompt => Tr(
        "There are unsaved changes.\n\nWould you like to save the changes before closing the image editor?",
        "当前有未保存的更改。\n\n关闭图片编辑器前，是否先保存这些更改？");

    public static string InsertImage => Tr("Insert image", "插入图片");
    public static string InsertImageCenter => Tr("Insert image in the center", "将图片插入到中央");
    public static string KeepCurrentCanvasSize => Tr("Keeps the current canvas size.", "保持当前画布大小不变。");
    public static string InsertImageBelow => Tr("Insert image below", "在下方插入图片");
    public static string ExpandCanvasDownward => Tr("Expands the canvas downward before adding the image.", "先向下扩展画布，再插入图片。");
    public static string InsertImageRightSide => Tr("Insert image on right side", "在右侧插入图片");
    public static string ExpandCanvasRight => Tr("Expands the canvas to the right before adding the image.", "先向右扩展画布，再插入图片。");
    public static string InsertImageDescription => Tr("Choose how to place the incoming image on the current canvas.", "选择如何将新图片放入当前画布。");
    public static string IncomingImageSummary(int width, int height) =>
        Format("Incoming image: {0} x {1}px", "新图片：{0} x {1}px", width, height);

    public static string CustomizeToolbar => Tr("Customize Toolbar", "自定义工具栏");
    public static string ResetToolbarToDefault => Tr("Reset toolbar to default", "恢复默认工具栏");
    public static string ToolbarHotkeyInputHint => Tr(
        "Click and press a shortcut. Backspace or Delete clears it.",
        "点击后直接按快捷键。按 Backspace 或 Delete 可清空。");
    public static string MoveUp => Tr("Move up", "上移");
    public static string MoveDown => Tr("Move down", "下移");
    public static string BeginGroup => Tr("Begin group", "作为新分组开始");
    public static string ShowHide => Tr("Show / Hide", "显示 / 隐藏");
    public static string Ok => Tr("OK", "确定");
    public static string InvalidHotkey(string itemName) => Format(
        "Invalid hotkey for {0}. Use formats like R, Ctrl+R, or Ctrl+Shift+R.",
        "{0} 的快捷键无效。请使用 R、Ctrl+R 或 Ctrl+Shift+R 这类格式。",
        itemName);
    public static string DuplicateHotkey(string itemName, string existingItemName, string hotkey) => Format(
        "{0} and {1} use the same hotkey ({2}).",
        "{0} 和 {1} 使用了相同的快捷键（{2}）。",
        itemName,
        existingItemName,
        hotkey);

    public static string Options => Tr("Options", "选项");
    public static string Close => Tr("Close", "关闭");
    public static string Dark => Tr("Dark", "深色");
    public static string Light => Tr("Light", "浅色");
    public static string FollowSystemTheme => Tr("Follow system theme", "跟随系统主题");
    public static string Theme => Tr("Theme", "主题");
    public static string FollowSystemAccentColor => Tr("Follow system accent color", "跟随系统强调色");
    public static string AccentColor => Tr("Accent color", "强调色");
    public static string RememberWindowState => Tr("Remember window state", "记住窗口状态");
    public static string ShowExitConfirmationOption => Tr("Show exit confirmation", "显示退出确认");
    public static string ZoomToFitOnOpen => Tr("Zoom to fit on open", "打开时自动适应窗口");
    public static string QuickCrop => Tr("Quick crop", "快速裁剪");
    public static string AutoCloseEditorOnTask => Tr("Auto close editor on task", "执行任务后自动关闭编辑器");
    public static string AutoCopyImageToClipboard => Tr("Auto copy image to clipboard", "自动复制图片到剪贴板");
    public static string ShowInsertImageDialog => Tr("Show insert image dialog", "显示插图对话框");
    public static string ShowNotifications => Tr("Show notifications", "显示通知");
    public static string CustomizeToolbarButton => Tr("Customize toolbar...", "自定义工具栏...");

    public static string Undo => Tr("Undo", "撤销");
    public static string Redo => Tr("Redo", "重做");
    public static string Delete => Tr("Delete", "删除");
    public static string DeleteAll => Tr("Delete all", "全部删除");
    public static string Flatten => Tr("Flatten", "合并图层");
    public static string Cut => Tr("Cut", "剪切");
    public static string Copy => Tr("Copy", "复制");
    public static string Paste => Tr("Paste", "粘贴");
    public static string Duplicate => Tr("Duplicate", "复制一份");
    public static string BringToFront => Tr("Bring to front", "移到最前");
    public static string BringForward => Tr("Bring forward", "上移一层");
    public static string SendBackward => Tr("Send backward", "下移一层");
    public static string SendToBack => Tr("Send to back", "移到最后");
    public static string HideToolbars => Tr("Hide toolbars", "隐藏工具栏");
    public static string ShowToolbars => Tr("Show toolbars", "显示工具栏");

    public static string Margin => Tr("Margin", "边距");
    public static string Padding => Tr("Padding", "内边距");
    public static string SmartPadding => Tr("Smart padding", "智能留白");
    public static string RoundedCorner => Tr("Rounded Corner", "圆角");
    public static string ShadowRadius => Tr("Shadow Radius", "阴影半径");
    public static string Ratio => Tr("Ratio", "比例");
    public static string Auto => Tr("Auto", "自动");
    public static string EditGradientColor1 => Tr("Edit gradient color 1", "编辑渐变颜色 1");
    public static string EditGradientColor2 => Tr("Edit gradient color 2", "编辑渐变颜色 2");
    public static string EditBackgroundColor => Tr("Edit background color", "编辑背景颜色");
    public static string BrowseBackgroundImage => Tr("Browse background image...", "选择背景图片...");
    public static string BackgroundImage => Tr("Background image", "背景图片");
    public static string SelectBackgroundImage => Tr("Select background image", "选择背景图片");
    public static string OpenImage => Tr("Open image", "打开图片");
    public static string SaveImageAsTitle => Tr("Save image as", "图片另存为");

    public static string ZoomTooltip => Tr("Zoom (Ctrl + Wheel)", "缩放（Ctrl + 滚轮）");
    public static string CancelTooltip => Tr("Cancel (Esc)", "取消（Esc）");
    public static string CopyImageTooltip => Tr("Copy image to clipboard (Ctrl+C)", "复制图片到剪贴板（Ctrl+C）");
    public static string SaveImageTooltip => Tr("Save image (Ctrl+S)", "保存图片（Ctrl+S）");
    public static string SaveImageAsTooltip => Tr("Save image as... (Ctrl+Shift+S)", "图片另存为...（Ctrl+Shift+S）");
    public static string PinImageToScreenTooltip => Tr("Pin image to screen (Ctrl+P)", "固定图片到屏幕（Ctrl+P）");
    public static string PrintImageTooltip => Tr("Print image... (Ctrl+Shift+P)", "打印图片...（Ctrl+Shift+P）");
    public static string UploadImageTooltip => Tr("Upload image (Ctrl+U)", "上传图片（Ctrl+U）");
    public static string ContinueTooltip => Tr("Continue (Enter)", "继续（Enter）");
    public static string RunAfterCaptureTasksTooltip => Tr("Run after capture tasks (Enter)", "执行截图后任务（Enter）");

    public static string NewFileMenuItem => Tr("New...", "新建...");
    public static string OpenFileMenuItem => Tr("Open...", "打开...");
    public static string OpenRecent => Tr("Open recent", "最近打开");
    public static string Save => Tr("Save", "保存");
    public static string SaveAs => Tr("Save as...", "另存为...");
    public static string File => Tr("File", "文件");
    public static string ImageEffects => Tr("Image Effects", "图片特效");
    public static string FavoriteImageEffectsHint => Tr("Favorite image effects (Right click)", "常用图片特效（右键管理）");

    public static string UndoTooltip => Tr("Undo (Ctrl+Z)", "撤销（Ctrl+Z）");
    public static string RedoTooltip => Tr("Redo (Ctrl+Y)", "重做（Ctrl+Y）");
    public static string DeleteTooltip => Tr("Delete (Delete)", "删除（Delete）");
    public static string DeleteAllTooltip => Tr("Delete all (Shift+Delete)", "全部删除（Shift+Delete）");
    public static string BorderColor => Tr("Border color", "边框颜色");
    public static string FillColor => Tr("Fill color", "填充颜色");
    public static string TextColor => Tr("Text color", "文字颜色");
    public static string Thickness => Tr("Thickness", "粗细");
    public static string BorderStyle => Tr("Border style", "边框样式");
    public static string CornerRadius => Tr("Corner radius", "圆角半径");
    public static string FontFamily => Tr("Font family", "字体");
    public static string HorizontalAlignment => Tr("Horizontal alignment", "水平对齐");
    public static string ArrowStyle => Tr("Arrow style", "箭头样式");
    public static string CursorType => Tr("Cursor type", "光标类型");
    public static string FontSize => Tr("Font size", "字号");
    public static string ToggleBold => Tr("Toggle bold", "切换粗体");
    public static string ToggleItalic => Tr("Toggle italic", "切换斜体");
    public static string StepType => Tr("Step type", "序号类型");
    public static string StartingNumber => Tr("Starting number", "起始编号");
    public static string EffectStrength => Tr("Effect strength", "效果强度");
    public static string BlurAmount => Tr("Blur amount", "模糊强度");
    public static string ToggleEllipseShape => Tr("Toggle ellipse shape", "切换椭圆形状");
    public static string ToggleTail => Tr("Toggle tail", "切换尾巴");
    public static string ToggleShadowWithOptions => Tr("Toggle shadow\nShadow options (Right click)", "切换阴影\n阴影选项（右键）");
    public static string Shadow => Tr("Shadow", "阴影");
    public static string Color => Tr("Color", "颜色");
    public static string Blur => Tr("Blur", "模糊");
    public static string Opacity => Tr("Opacity", "不透明度");
    public static string OffsetX => Tr("Offset X", "X 偏移");
    public static string OffsetY => Tr("Offset Y", "Y 偏移");
    public static string ShadowColor => Tr("Shadow color", "阴影颜色");
    public static string PickColorFromScreen => Tr("Pick color from screen", "从屏幕取色");
    public static string ZoomToFit => Tr("Zoom to Fit", "适应窗口");

    public static string SearchImageEffectsPlaceholder => Tr("Search image effects...", "搜索图片特效...");
    public static string SearchImageEffects(int count) => Format("Search image effects... ({0})", "搜索图片特效...（{0}）", count);
    public static string RecentHeaderHint => Tr("Right-click an effect item to remove it from Recent.", "右键点击特效项可将其从最近使用中移除。");
    public static string FavoritesHeaderHint => Tr("Right-click an effect item to add or remove it from Favorites.", "右键点击特效项可添加到或移出常用。");
    public static string TranslateEffectCategory(string english) => english switch
    {
        "Recent" => Tr("Recent", "最近使用"),
        "Favorites" => Tr("Favorites", "常用"),
        "Manipulations" => Tr("Manipulations", "变换"),
        "Adjustments" => Tr("Adjustments", "调整"),
        "Filters" => Tr("Filters", "滤镜"),
        "Drawings" => Tr("Drawings", "绘制"),
        _ => english
    };

    public static string BackgroundModeGradient => Tr("Gradient", "渐变");
    public static string BackgroundModeColor => Tr("Color", "纯色");
    public static string BackgroundModeImage => Tr("Image", "图片");
    public static string BackgroundModeWallpaper => Tr("Wallpaper", "壁纸");

    public static string TranslateBackgroundMode(string english) => english switch
    {
        "Gradient" => BackgroundModeGradient,
        "Color" => BackgroundModeColor,
        "Transparent" => Transparent,
        "Image" => BackgroundModeImage,
        "Wallpaper" => BackgroundModeWallpaper,
        _ => english
    };

    public static string BackgroundRemoverGuideTooltip => Tr("Open guide page...", "打开指南页面...");
    public static string RefreshModelsTooltip => Tr("Refresh models", "刷新模型");
    public static string OpenModelsFolderTooltip => Tr("Open models folder", "打开模型文件夹");
    public static string BackgroundRemovalModel => Tr("Background removal model", "背景移除模型");
    public static string ProcessingDevice => Tr("Processing device", "处理设备");
    public static string ImageLabel => Tr("Image", "图片");
    public static string NoImageSelected => Tr("No image selected", "未选择图片");
    public static string Browse => Tr("Browse...", "浏览...");
    public static string Models => Tr("Models", "模型");
    public static string Device => Tr("Device", "设备");
    public static string RemoveBackground => Tr("Remove Background", "移除背景");
    public static string SelectImagePreviewHere => Tr("Select an image to preview it here.", "选择一张图片后会在这里预览。");
    public static string SelectImage => Tr("Select image", "选择图片");
    public static string BackgroundRemovedIn(long milliseconds) => Format("Background removed in {0} ms.", "背景已移除，耗时 {0} 毫秒。", milliseconds);
    public static string ImageSavedNotification(string path) => Format("Image saved.\nFile path: {0}", "图片已保存。\n文件路径：{0}", path);
    public static string SelectParameter(string label) => Format("Select {0}", "选择{0}", label);

    public static string LoadingEmojis => Tr("Loading emojis...", "正在加载表情...");
    public static string EmojiPickerLoadingReady => Tr(
        "The picker is ready. Emoji previews are loading in the background.",
        "选择器已就绪，表情预览正在后台继续加载。");
    public static string NoEmojiMatches => Tr("No emojis match this search.", "没有匹配当前搜索的表情。");
    public static string TryDifferentKeyword => Tr("Try a different keyword.", "试试其他关键词。");
    public static string SearchEmojisWatermark(int categoryCount) => Format("Search emojis... ({0})", "搜索表情...（{0}）", categoryCount);
    public static string BrowseEmojis => Tr("Browse emojis", "浏览表情");
    public static string SearchResults(int count) => Format("Search results • {0} matches", "搜索结果 • {0} 个匹配项", count);

    public static string Image1 => Tr("Image 1", "图片 1");
    public static string Image2 => Tr("Image 2", "图片 2");
    public static string Choose => Tr("Choose...", "选择...");
    public static string Slider => Tr("Slider", "滑动对比");
    public static string DiffView => Tr("Diff View", "差异视图");
    public static string SelectImagesForComparison => Tr("Select Image 1 and Image 2 to show the comparison preview.", "请选择图片 1 和图片 2 以显示对比预览。");
    public static string Similarity(double value) => Format("Similarity: {0:0.##}%", "相似度：{0:0.##}%", value);
    public static string SimilarityUnknown => Tr("Similarity: -", "相似度：-");
    public static string SelectTwoImagesToCompare => Tr("Select two images to compare.", "请选择两张图片进行比较。");
    public static string ImagePickerUnavailable => Tr("Image picker is unavailable.", "图片选择器不可用。");
    public static string SelectImageTitle(int imageNumber) => Format("Select Image {0}", "选择图片 {0}", imageNumber);
    public static string SelectedFileNotImage => Tr("The selected file could not be loaded as an image.", "所选文件无法作为图片载入。");
    public static string FailedToLoadImage(string message) => Format("Failed to load image: {0}", "载入图片失败：{0}", message);
    public static string MatchingPixels(long matching, long total) => Format("{0:N0} of {1:N0} pixels match.", "{0:N0} / {1:N0} 像素一致。", matching, total);

    public static string ImageOpened => Tr("Image opened.", "图片已打开。");
    public static string NewImageCreated => Tr("New image created.", "已新建图片。");
    public static string ImageCropped => Tr("Image cropped.", "图片已裁剪。");
    public static string ImageCutOut => Tr("Image cut out.", "图片已抠出。");
    public static string ImageInserted => Tr("Image inserted.", "图片已插入。");
    public static string ImageAutoCropped => Tr("Image auto-cropped.", "图片已自动裁剪。");
    public static string ImageResized => Tr("Image resized.", "图片尺寸已调整。");
    public static string CanvasResized => Tr("Canvas resized.", "画布尺寸已调整。");
    public static string ImageRotatedClockwise => Tr("Image rotated 90 degrees clockwise.", "图片已顺时针旋转 90 度。");
    public static string ImageRotatedCounterClockwise => Tr("Image rotated 90 degrees counterclockwise.", "图片已逆时针旋转 90 度。");
    public static string ImageRotated180 => Tr("Image rotated 180 degrees.", "图片已旋转 180 度。");
    public static string ImageRotatedBy(string angleText) => Format("Image rotated by {0} degrees.", "图片已旋转 {0} 度。", angleText);
    public static string ImageFlippedHorizontally => Tr("Image flipped horizontally.", "图片已水平翻转。");
    public static string ImageFlippedVertically => Tr("Image flipped vertically.", "图片已垂直翻转。");
    public static string ImageEffectApplied => Tr("Image effect applied.", "图片特效已应用。");
    public static string ImageSavedToFile => Tr("Image saved to file.", "图片已保存到文件。");
    public static string FilePathLabel => Tr("File path: {0}", "文件路径：{0}");
    public static string SizeLabel(int width, int height) => Format("Size: {0}x{1}", "尺寸：{0}x{1}", width, height);
    public static string NoImage => Tr("No image", "无图片");
    public static string ImageCopiedToClipboard => Tr("Image copied to clipboard.", "图片已复制到剪贴板。");
    public static string ImagePrinted => Tr("Image printed.", "图片已打印。");
    public static string ImagePinnedToScreen => Tr("Image pinned to screen.", "图片已固定到屏幕。");
    public static string ImageUploading => Tr("Image is uploading.", "图片正在上传。");
    public static string AppliedEffect(string name) => Format("Applied {0}", "已应用 {0}", name);

    public static string InvertedColors => Tr("Inverted colors", "颜色已反相");
    public static string AppliedBlackAndWhiteFilter => Tr("Applied Black & White filter", "已应用黑白滤镜");
    public static string AppliedSepiaFilter => Tr("Applied Sepia filter", "已应用棕褐色滤镜");
    public static string AppliedPolaroidFilter => Tr("Applied Polaroid filter", "已应用拍立得滤镜");
    public static string AppliedEdgeDetectFilter => Tr("Applied Edge detect filter", "已应用边缘检测滤镜");
    public static string AppliedEmbossFilter => Tr("Applied Emboss filter", "已应用浮雕滤镜");
    public static string AppliedMeanRemovalFilter => Tr("Applied Mean removal filter", "已应用均值去除滤镜");
    public static string AppliedSmoothFilter => Tr("Applied Smooth filter", "已应用平滑滤镜");

    public static string TranslateArrowStyle(string english) => english switch
    {
        "Classic" => Tr("Classic", "经典"),
        "Double" => Tr("Double", "双箭头"),
        "Modern" => Tr("Modern", "现代"),
        "Basic" => Tr("Basic", "基础"),
        "Line" => Tr("Line", "线条"),
        _ => english
    };

    public static string TranslateTheme(string english) => english switch
    {
        "Dark" => Dark,
        "Light" => Light,
        _ => english
    };

    public static string TranslateEffectLabel(string english) => english switch
    {
        "Auto crop image" => "自动裁剪图片",
        "Crop image" => "裁剪图片",
        "Resize image" => "调整图片尺寸",
        "Resize canvas" => "调整画布尺寸",
        "Rotate 90 clockwise" => "顺时针旋转 90 度",
        "Rotate 90 counter-clockwise" => "逆时针旋转 90 度",
        "Rotate 180" => "旋转 180 度",
        "Rotate custom angle" => "自定义旋转角度",
        "Flip horizontal" => "水平翻转",
        "Flip vertical" => "垂直翻转",
        "Invert colors" => "反相颜色",
        "Black & White" => "黑白",
        "Sepia" => "棕褐色",
        "Polaroid" => "拍立得",
        "Edge detect" => "边缘检测",
        "Emboss" => "浮雕",
        "Mean removal" => "均值去除",
        "Smooth" => "平滑",
        "Blur" => "模糊",
        "Pixelate" => "像素化",
        "Shadow" => "阴影",
        "Border" => "边框",
        "Reflection" => "倒影",
        "Vignette" => "暗角",
        "Remove background" => "移除背景",
        "Text watermark" => "文字水印",
        "Text" => "文字",
        "Shape" => "形状",
        "Line" => "线条",
        "Image" => "图片",
        "Background" => "背景",
        "Background image" => "背景图片",
        _ => english
    };

    public static string TranslateEffectDescription(string english) => english switch
    {
        "Automatically crops the image." => "自动裁剪图片。",
        "Crops the image." => "裁剪图片。",
        "Resizes the image." => "调整图片尺寸。",
        "Resizes the canvas." => "调整画布尺寸。",
        "Rotates the image 90 degrees clockwise." => "将图片顺时针旋转 90 度。",
        "Rotates the image 90 degrees counter-clockwise." => "将图片逆时针旋转 90 度。",
        "Rotates the image 180 degrees." => "将图片旋转 180 度。",
        "Rotates the image by a custom angle." => "按自定义角度旋转图片。",
        "Flips the image horizontally." => "水平翻转图片。",
        "Flips the image vertically." => "垂直翻转图片。",
        "Converts image to negative by inverting colors." => "通过颜色反相将图片转换为负片效果。",
        "Applies a black and white filter." => "应用黑白滤镜。",
        "Applies a sepia tone effect." => "应用棕褐色调效果。",
        "Applies a Polaroid filter." => "应用拍立得滤镜。",
        "Applies an edge detection filter." => "应用边缘检测滤镜。",
        "Applies an emboss filter." => "应用浮雕滤镜。",
        "Applies a mean removal filter." => "应用均值去除滤镜。",
        "Applies a smoothing effect." => "应用平滑效果。",
        "Applies a blur effect." => "应用模糊效果。",
        "Pixelates the image." => "对图片进行像素化处理。",
        "Adds a drop shadow to the image." => "为图片添加投影。",
        "Adds a border to the image." => "为图片添加边框。",
        "Adds a reflection to the bottom of the image." => "在图片底部添加倒影。",
        "Applies a vignette effect." => "应用暗角效果。",
        "Removes border-connected background colors and turns them transparent." => "移除与边缘相连的背景颜色并将其变为透明。",
        "Draws a text watermark with background on the image." => "在图片上绘制带背景的文字水印。",
        "Draws text on the image." => "在图片上绘制文字。",
        "Draws a shape on the image." => "在图片上绘制形状。",
        "Draws a line on the image." => "在图片上绘制线条。",
        "Draws an image overlay on the source image." => "在原图上绘制图片叠加层。",
        "Draws a solid color background behind the image." => "在图片后方绘制纯色背景。",
        "Draws a background image behind the source image." => "在原图后方绘制背景图片。",
        _ => english
    };
}
