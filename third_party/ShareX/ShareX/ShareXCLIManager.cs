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

using ShareX.HelpersLib;
using ShareX.ImageEditor.Core.Annotations;
using ShareX.ImageEditor.Hosting;
using EditorStepType = ShareX.ImageEditor.Core.Annotations.StepType;
using Newtonsoft.Json;
using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Threading.Tasks;

namespace ShareX
{
    public class ShareXCLIManager : CLIManager
    {
        public ShareXCLIManager(string[] arguments) : base(arguments)
        {
        }

        public async Task UseCommandLineArgs()
        {
            await UseCommandLineArgs(Commands);
        }

        public async Task UseCommandLineArgs(List<CLICommand> commands)
        {
            if (commands != null && commands.Count > 0)
            {
                if (Program.KaixinIntegration)
                {
                    // The bundled integration accepts only its two local capture
                    // commands. Ignore generic ShareX CLI upload/download commands,
                    // including arguments delivered later through the instance pipe.
                    CLICommand outputCommand = commands.FirstOrDefault(x => x.CheckCommand("KaixinOutputPath"));
                    string outputPath = NormalizeKaixinOutputPath(outputCommand?.Parameter);
                    CLICommand resultCommand = commands.FirstOrDefault(x => x.CheckCommand("KaixinResultPath"));
                    string resultPath = NormalizeKaixinResultPath(resultCommand?.Parameter);
                    CLICommand optionsCommand = commands.FirstOrDefault(x => x.CheckCommand("KaixinOptionsPath"));
                    KaixinCaptureOptions options = LoadKaixinCaptureOptions(optionsCommand?.Parameter);
                    foreach (CLICommand command in commands)
                    {
                        if (command.IsCommand &&
                            (command.CheckCommand("KaixinRectangleRegion") || command.CheckCommand("KaixinCaptureWindow")))
                        {
                            DebugHelper.WriteLine("Kaixin command line: " + command);
                            await CheckCLIHotkey(command, outputPath, resultPath, options);
                        }
                    }

                    return;
                }

                TaskSettings taskSettings = FindCLITask(commands);

                foreach (CLICommand command in commands)
                {
                    DebugHelper.WriteLine("CommandLine: " + command);

                    if (command.IsCommand)
                    {
                        if (CheckCustomUploader(command) || CheckImageEffect(command) || await CheckCLIHotkey(command) || await CheckCLIWorkflow(command) ||
                            await CheckNativeMessagingInput(command))
                        {
                        }

                        continue;
                    }

                    if (URLHelpers.IsValidURL(command.Command))
                    {
                        UploadManager.DownloadAndUploadFile(command.Command, taskSettings);
                    }
                    else
                    {
                        UploadManager.UploadFile(command.Command, taskSettings);
                    }
                }
            }
        }

        private TaskSettings FindCLITask(List<CLICommand> commands)
        {
            if (Program.HotkeysConfig != null)
            {
                CLICommand command = commands.FirstOrDefault(x => x.CheckCommand("task") && !string.IsNullOrEmpty(x.Parameter));

                if (command != null)
                {
                    foreach (HotkeySettings hotkeySetting in Program.HotkeysConfig.Hotkeys)
                    {
                        if (command.Parameter == hotkeySetting.TaskSettings.ToString())
                        {
                            return TaskSettings.GetSafeTaskSettings(hotkeySetting.TaskSettings);
                        }
                    }
                }
            }

            return null;
        }

        private bool CheckCustomUploader(CLICommand command)
        {
            if (command.Command.Equals("CustomUploader", StringComparison.OrdinalIgnoreCase))
            {
                if (!string.IsNullOrEmpty(command.Parameter) && command.Parameter.EndsWith(".sxcu", StringComparison.OrdinalIgnoreCase))
                {
                    TaskHelpers.ImportCustomUploader(command.Parameter);
                }

                return true;
            }

            return false;
        }

        private bool CheckImageEffect(CLICommand command)
        {
            if (command.Command.Equals("ImageEffect", StringComparison.OrdinalIgnoreCase))
            {
                if (!string.IsNullOrEmpty(command.Parameter) && command.Parameter.EndsWith(".sxie", StringComparison.OrdinalIgnoreCase))
                {
                    TaskHelpers.ImportImageEffect(command.Parameter);
                }

                return true;
            }

            return false;
        }

        private async Task<bool> CheckCLIHotkey(CLICommand command, string kaixinOutputPath = null, string kaixinResultPath = null,
            KaixinCaptureOptions kaixinOptions = null)
        {
            if (command.CheckCommand("KaixinRectangleRegion"))
            {
                await TaskHelpers.ExecuteJob(CreateKaixinCaptureTaskSettings(kaixinOutputPath, kaixinResultPath, kaixinOptions), HotkeyType.RectangleRegion);
                return true;
            }

            if (command.CheckCommand("KaixinCaptureWindow"))
            {
                if (long.TryParse(command.Parameter, out long windowHandle) && windowHandle != 0)
                {
                    new CaptureWindow(new IntPtr(windowHandle)).Capture(CreateKaixinCaptureTaskSettings(
                        kaixinOutputPath, kaixinResultPath, kaixinOptions));
                }

                return true;
            }

            foreach (HotkeyType job in Helpers.GetEnums<HotkeyType>())
            {
                if (command.CheckCommand(job.ToString()))
                {
                    string filePath = null;

                    try
                    {
                        filePath = CheckParameterForFilePath(command);
                    }
                    catch (Exception e)
                    {
                        DebugHelper.WriteException(e);

                        return true;
                    }

                    await TaskHelpers.ExecuteJob(job, filePath);

                    return true;
                }
            }

            return false;
        }

        private static TaskSettings CreateKaixinCaptureTaskSettings(string outputPath, string resultPath, KaixinCaptureOptions options)
        {
            options ??= new KaixinCaptureOptions();
            EImageFormat imageFormat = string.Equals(Path.GetExtension(outputPath), ".jpg", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(Path.GetExtension(outputPath), ".jpeg", StringComparison.OrdinalIgnoreCase)
                ? EImageFormat.JPEG
                : EImageFormat.PNG;

            // Use ShareX's persisted default tool settings instead of creating
            // a fresh ImageEditorOptions instance for every Kaixin capture.
            // The editor mutates this shared options object as the user changes
            // tools, colors, thickness and window state, allowing those choices
            // to survive both the next capture and the next ShareX process.
            Program.DefaultTaskSettings.ToolsSettings.UseLegacyImageEditor = false;
            Program.DefaultTaskSettings.ToolsSettings.ShowImageEditorSelector = false;
            ApplyKaixinEditorOptions(options);
            AfterCaptureTasks afterCaptureTasks = AfterCaptureTasks.CopyImageToClipboard;
            if (options.OpenEditor)
            {
                afterCaptureTasks |= AfterCaptureTasks.AnnotateImage;
            }
            if (options.PinToScreen)
            {
                afterCaptureTasks |= AfterCaptureTasks.PinToScreen;
            }
            if (options.OpenFolderAfterCapture && !string.IsNullOrWhiteSpace(outputPath))
            {
                afterCaptureTasks |= AfterCaptureTasks.ShowInExplorer;
            }

            TaskSettingsCapture captureSettings = new TaskSettingsCapture
            {
                ShowCursor = options.ShowCursor,
                ScreenshotDelay = (decimal)Math.Clamp(options.ScreenshotDelay, 0, 60),
                CaptureClientArea = options.CaptureClientArea,
                CaptureShadow = options.CaptureShadow,
                CaptureAutoHideTaskbar = options.HideTaskbar,
                CaptureAutoHideDesktopIcons = options.HideDesktopIcons
            };
            captureSettings.SurfaceOptions.DetectWindows = options.DetectWindows;
            captureSettings.SurfaceOptions.DetectControls = options.DetectControls;
            captureSettings.SurfaceOptions.ShowMagnifier = options.ShowMagnifier;
            captureSettings.SurfaceOptions.MagnifierPixelCount = Math.Clamp(options.MagnifierPixelCount, 1, 100);
            captureSettings.SurfaceOptions.MagnifierPixelSize = Math.Clamp(options.MagnifierPixelSize, 40, 1000);
            captureSettings.SurfaceOptions.UseSquareMagnifier = options.MagnifierSquare;
            captureSettings.SurfaceOptions.ShowCenterCrosshair = options.ShowCenterCrosshair;
            captureSettings.SurfaceOptions.ShowInfo = options.ShowInfo;
            captureSettings.SurfaceOptions.ShowCrosshair = options.ShowCrosshair;
            captureSettings.SurfaceOptions.UseDimming = options.UseDimming;
            captureSettings.SurfaceOptions.BackgroundDimStrength = Math.Clamp(options.DimStrength, 0, 100);
            captureSettings.SurfaceOptions.EnableAnimations = options.EnableAnimations;
            captureSettings.SurfaceOptions.IsFixedSize = options.FixedSizeEnabled;
            captureSettings.SurfaceOptions.FixedSize = new Size(
                Math.Clamp(options.FixedWidth, 1, 16384),
                Math.Clamp(options.FixedHeight, 1, 16384));

            return new TaskSettings
            {
                Description = "Kaixin input method capture",
                KaixinOutputPath = outputPath ?? "",
                KaixinResultPath = resultPath ?? "",
                UseDefaultAfterCaptureJob = false,
                AfterCaptureJob = afterCaptureTasks,
                // Use explicit per-request completion feedback. Never inherit
                // ShareX sound settings for an embedded capture request.
                UseDefaultGeneralSettings = false,
                GeneralSettings = new TaskSettingsGeneral
                {
                    PlaySoundAfterCapture = false,
                    PlaySoundAfterUpload = false,
                    PlaySoundAfterAction = false,
                    ShowToastNotificationAfterTaskCompleted = options.ShowNotification
                },
                UseDefaultImageSettings = false,
                ImageSettings = new TaskSettingsImage
                {
                    ImageFormat = imageFormat,
                    ImageAutoUseJPEG = false,
                    ImageJPEGQuality = Math.Clamp(options.JpegQuality, 1, 100)
                },
                UseDefaultCaptureSettings = false,
                CaptureSettings = captureSettings,
                UseDefaultToolsSettings = true
            };
        }

        private static KaixinCaptureOptions LoadKaixinCaptureOptions(string optionsPath)
        {
            string normalizedPath = NormalizeKaixinOptionsPath(optionsPath);
            if (string.IsNullOrEmpty(normalizedPath))
            {
                return new KaixinCaptureOptions();
            }

            try
            {
                KaixinCaptureOptions options = JsonConvert.DeserializeObject<KaixinCaptureOptions>(
                    File.ReadAllText(normalizedPath));
                if (options == null || options.Version != 1)
                {
                    DebugHelper.WriteLine("Kaixin capture options rejected (unsupported version).");
                    return new KaixinCaptureOptions();
                }
                return options;
            }
            catch (Exception e)
            {
                DebugHelper.WriteException(e, "Kaixin capture options could not be loaded.");
                return new KaixinCaptureOptions();
            }
        }

        private static string NormalizeKaixinOptionsPath(string optionsPath)
        {
            if (string.IsNullOrWhiteSpace(optionsPath))
            {
                return null;
            }

            try
            {
                string fullPath = Path.GetFullPath(optionsPath);
                string allowedRoot = Path.GetFullPath(Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                    "kaixin", "sharex-results"));
                string allowedPrefix = allowedRoot.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar) + Path.DirectorySeparatorChar;
                if (!fullPath.EndsWith(".options.json", StringComparison.OrdinalIgnoreCase) ||
                    !fullPath.StartsWith(allowedPrefix, StringComparison.OrdinalIgnoreCase))
                {
                    DebugHelper.WriteLine("Kaixin options path rejected: " + fullPath);
                    return null;
                }
                return fullPath;
            }
            catch (Exception e)
            {
                DebugHelper.WriteException(e, "Kaixin options path rejected.");
                return null;
            }
        }

        private static void ApplyKaixinEditorOptions(KaixinCaptureOptions options)
        {
            ImageEditorOptions editor = Program.DefaultTaskSettings.ToolsSettings.ImageEditorOptions;
            string color = NormalizeEditorColor(options.EditorAnnotationColor, "#FFF23C3C");
            editor.BorderColorHex = color;
            editor.StepFillColorHex = color;
            editor.TextTextColorHex = NormalizeEditorColor(options.EditorTextColor, "#FFFFFFFF");
            editor.TextBorderColorHex = NormalizeEditorColor(options.EditorTextBorderColor, color);
            editor.Thickness = Math.Clamp(options.EditorThickness, 1, 100);
            editor.TextThickness = Math.Clamp(options.EditorThickness * 2, 1, 100);
            editor.TextFontFamily = string.IsNullOrWhiteSpace(options.EditorFontFamily)
                ? "Segoe UI"
                : options.EditorFontFamily.Trim();
            editor.SpeechBalloonFontFamily = editor.TextFontFamily;
            editor.TextFontSize = (float)Math.Clamp(options.EditorFontSize, 6, 200);
            editor.SpeechBalloonFontSize = editor.TextFontSize;
            editor.BlurStrength = (float)Math.Clamp(options.EditorBlurStrength, 1, 100);
            editor.PixelateStrength = (float)Math.Clamp(options.EditorPixelateStrength, 1, 100);
            editor.AutoCloseEditorOnTask = options.EditorAutoClose;
            editor.ShowNotifications = options.ShowNotification;

            string arrowToken = NormalizeEnumToken(options.EditorArrowStyle);
            if (Enum.TryParse(arrowToken, true, out ArrowStyle arrowStyle))
            {
                editor.ArrowStyle = arrowStyle;
            }
            string stepToken = NormalizeEnumToken(options.EditorStepType);
            if (Enum.TryParse(stepToken, true, out EditorStepType stepType))
            {
                editor.StepType = stepType;
            }
            if (!options.EditorRememberLastTool)
            {
                string toolToken = NormalizeEnumToken(options.EditorDefaultTool);
                if (Enum.TryParse(toolToken, true, out EditorTool tool))
                {
                    editor.LastUsedAnnotationTool = tool;
                }
            }

            HashSet<string> visibleTools = new HashSet<string>(
                (options.EditorToolbarTools ?? "").Split(',', StringSplitOptions.RemoveEmptyEntries)
                    .Select(value => value.Trim()),
                StringComparer.OrdinalIgnoreCase);
            List<ImageEditorToolbarItemOptions> toolbarItems = new List<ImageEditorToolbarItemOptions>
            {
                new ImageEditorToolbarItemOptions { Id = "File", IsVisible = true, Hotkey = null }
            };
            foreach (EditorTool tool in Enum.GetValues<EditorTool>())
            {
                toolbarItems.Add(new ImageEditorToolbarItemOptions
                {
                    Id = tool.ToString(),
                    IsVisible = visibleTools.Contains(tool.ToString()),
                    Hotkey = null
                });
            }
            foreach (string panel in new[] { "Background", "ImageEffects" })
            {
                toolbarItems.Add(new ImageEditorToolbarItemOptions
                {
                    Id = panel,
                    IsVisible = visibleTools.Contains(panel),
                    Hotkey = null
                });
            }
            editor.ToolbarItems = toolbarItems;
        }

        private static string NormalizeEditorColor(string value, string fallback)
        {
            string color = (value ?? "").Trim();
            if (color.Length == 7 && color[0] == '#' && color.Skip(1).All(Uri.IsHexDigit))
            {
                return "#FF" + color.Substring(1).ToUpperInvariant();
            }
            if (color.Length == 9 && color[0] == '#' && color.Skip(1).All(Uri.IsHexDigit))
            {
                return color.ToUpperInvariant();
            }
            return fallback;
        }

        private static string NormalizeEnumToken(string value)
        {
            return (value ?? "").Replace("_", "").Replace("-", "").Trim();
        }

        private static string NormalizeKaixinOutputPath(string outputPath)
        {
            if (string.IsNullOrWhiteSpace(outputPath))
            {
                return null;
            }

            try
            {
                string fullPath = Path.GetFullPath(outputPath);
                string extension = Path.GetExtension(fullPath);
                if (!extension.Equals(".png", StringComparison.OrdinalIgnoreCase) &&
                    !extension.Equals(".jpg", StringComparison.OrdinalIgnoreCase) &&
                    !extension.Equals(".jpeg", StringComparison.OrdinalIgnoreCase))
                {
                    DebugHelper.WriteLine("Kaixin output path rejected (unsupported extension): " + fullPath);
                    return null;
                }

                return fullPath;
            }
            catch (Exception e)
            {
                DebugHelper.WriteException(e, "Kaixin output path rejected.");
                return null;
            }
        }

        private sealed class KaixinCaptureOptions
        {
            public int Version { get; set; } = 1;
            public bool OpenEditor { get; set; } = true;
            public bool DetectWindows { get; set; } = true;
            public bool DetectControls { get; set; } = true;
            public bool ShowMagnifier { get; set; } = true;
            public int MagnifierPixelCount { get; set; } = 15;
            public int MagnifierPixelSize { get; set; } = 160;
            public bool MagnifierSquare { get; set; }
            public bool ShowCenterCrosshair { get; set; }
            public bool ShowInfo { get; set; } = true;
            public bool ShowCrosshair { get; set; }
            public bool UseDimming { get; set; } = true;
            public int DimStrength { get; set; } = 20;
            public bool EnableAnimations { get; set; } = true;
            public bool FixedSizeEnabled { get; set; }
            public int FixedWidth { get; set; } = 250;
            public int FixedHeight { get; set; } = 250;
            public bool ShowCursor { get; set; } = true;
            public double ScreenshotDelay { get; set; }
            public bool CaptureClientArea { get; set; }
            public bool CaptureShadow { get; set; } = true;
            public bool HideTaskbar { get; set; }
            public bool HideDesktopIcons { get; set; }
            public int JpegQuality { get; set; } = 90;
            public bool OpenFolderAfterCapture { get; set; }
            public bool PinToScreen { get; set; }
            public bool ShowNotification { get; set; }
            public string EditorAnnotationColor { get; set; } = "#F23C3C";
            public string EditorTextColor { get; set; } = "#FFFFFF";
            public string EditorTextBorderColor { get; set; } = "#F23C3C";
            public int EditorThickness { get; set; } = 4;
            public string EditorFontFamily { get; set; } = "Segoe UI";
            public double EditorFontSize { get; set; } = 48;
            public string EditorArrowStyle { get; set; } = "classic";
            public double EditorBlurStrength { get; set; } = 30;
            public double EditorPixelateStrength { get; set; } = 20;
            public string EditorStepType { get; set; } = "numeric";
            public bool EditorAutoClose { get; set; }
            public bool EditorRememberLastTool { get; set; } = true;
            public string EditorDefaultTool { get; set; } = "rectangle";
            public string EditorToolbarTools { get; set; } =
                "Select,Rectangle,Ellipse,Line,Arrow,Freehand,Text,SpeechBalloon,Step,Image,Emoji,Cursor,Highlight,SmartEraser,Blur,Pixelate,Magnify,Spotlight,Crop,CutOut,Background,ImageEffects";
        }

        private static string NormalizeKaixinResultPath(string resultPath)
        {
            if (string.IsNullOrWhiteSpace(resultPath))
            {
                return null;
            }

            try
            {
                string fullPath = Path.GetFullPath(resultPath);
                string allowedRoot = Path.GetFullPath(Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                    "kaixin", "sharex-results"));
                string allowedPrefix = allowedRoot.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar) + Path.DirectorySeparatorChar;
                if (!Path.GetExtension(fullPath).Equals(".result", StringComparison.OrdinalIgnoreCase) ||
                    !fullPath.StartsWith(allowedPrefix, StringComparison.OrdinalIgnoreCase))
                {
                    DebugHelper.WriteLine("Kaixin result path rejected: " + fullPath);
                    return null;
                }

                return fullPath;
            }
            catch (Exception e)
            {
                DebugHelper.WriteException(e, "Kaixin result path rejected.");
                return null;
            }
        }

        private string CheckParameterForFilePath(CLICommand command)
        {
            if (command != null && !string.IsNullOrEmpty(command.Parameter))
            {
                string filePath = FileHelpers.GetAbsolutePath(command.Parameter);

                if (!File.Exists(filePath))
                {
                    throw new FileNotFoundException();
                }

                return filePath;
            }

            return null;
        }

        private async Task<bool> CheckCLIWorkflow(CLICommand command)
        {
            if (Program.HotkeysConfig != null && command.CheckCommand("workflow") && !string.IsNullOrEmpty(command.Parameter))
            {
                foreach (HotkeySettings hotkeySetting in Program.HotkeysConfig.Hotkeys)
                {
                    if (hotkeySetting.TaskSettings.Job != HotkeyType.None)
                    {
                        if (command.Parameter == hotkeySetting.TaskSettings.ToString())
                        {
                            await TaskHelpers.ExecuteJob(hotkeySetting.TaskSettings);

                            return true;
                        }
                    }
                }
            }

            return false;
        }

        private async Task<bool> CheckNativeMessagingInput(CLICommand command)
        {
            if (command.Command.Equals("NativeMessagingInput", StringComparison.OrdinalIgnoreCase))
            {
                if (!string.IsNullOrEmpty(command.Parameter) && command.Parameter.EndsWith(".json", StringComparison.OrdinalIgnoreCase))
                {
                    await TaskHelpers.HandleNativeMessagingInput(command.Parameter);
                }

                return true;
            }

            return false;
        }
    }
}
