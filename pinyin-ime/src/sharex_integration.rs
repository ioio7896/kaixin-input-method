use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const SHAREX_EXE: &str = "KaixinShareX.exe";
// Keep the protocol version in both the pipe and ShareX mutex. Otherwise a
// process left running by an older installation can accept a newer private
// command while silently ignoring its output path or post-capture options.
const SHAREX_PIPE_SUFFIX: &str = "ShareX-KaixinIntegration-V3";
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ShareXCaptureOptions {
    pub version: u32,
    pub open_editor: bool,
    pub detect_windows: bool,
    pub detect_controls: bool,
    pub show_magnifier: bool,
    pub magnifier_pixel_count: usize,
    pub magnifier_pixel_size: usize,
    pub magnifier_square: bool,
    pub show_center_crosshair: bool,
    pub show_info: bool,
    pub show_crosshair: bool,
    pub use_dimming: bool,
    pub dim_strength: usize,
    pub enable_animations: bool,
    pub fixed_size_enabled: bool,
    pub fixed_width: usize,
    pub fixed_height: usize,
    pub show_cursor: bool,
    pub screenshot_delay: f64,
    pub capture_client_area: bool,
    pub capture_shadow: bool,
    pub hide_taskbar: bool,
    pub hide_desktop_icons: bool,
    pub jpeg_quality: usize,
    pub open_folder_after_capture: bool,
    pub pin_to_screen: bool,
    pub show_notification: bool,
    pub editor_annotation_color: String,
    pub editor_text_color: String,
    pub editor_text_border_color: String,
    pub editor_thickness: usize,
    pub editor_font_family: String,
    pub editor_font_size: f64,
    pub editor_arrow_style: String,
    pub editor_blur_strength: f64,
    pub editor_pixelate_strength: f64,
    pub editor_step_type: String,
    pub editor_auto_close: bool,
    pub editor_remember_last_tool: bool,
    pub editor_default_tool: String,
    pub editor_toolbar_tools: String,
}

impl Default for ShareXCaptureOptions {
    fn default() -> Self {
        Self {
            version: 1,
            open_editor: true,
            detect_windows: true,
            detect_controls: true,
            show_magnifier: true,
            magnifier_pixel_count: 15,
            magnifier_pixel_size: 160,
            magnifier_square: false,
            show_center_crosshair: false,
            show_info: true,
            show_crosshair: false,
            use_dimming: true,
            dim_strength: 20,
            enable_animations: true,
            fixed_size_enabled: false,
            fixed_width: 250,
            fixed_height: 250,
            show_cursor: true,
            screenshot_delay: 0.0,
            capture_client_area: false,
            capture_shadow: true,
            hide_taskbar: false,
            hide_desktop_icons: false,
            jpeg_quality: 90,
            open_folder_after_capture: false,
            pin_to_screen: false,
            show_notification: false,
            editor_annotation_color: "#F23C3C".to_string(),
            editor_text_color: "#FFFFFF".to_string(),
            editor_text_border_color: "#F23C3C".to_string(),
            editor_thickness: 4,
            editor_font_family: "Segoe UI".to_string(),
            editor_font_size: 48.0,
            editor_arrow_style: "classic".to_string(),
            editor_blur_strength: 30.0,
            editor_pixelate_strength: 20.0,
            editor_step_type: "numeric".to_string(),
            editor_auto_close: false,
            editor_remember_last_tool: true,
            editor_default_tool: "rectangle".to_string(),
            editor_toolbar_tools: "Select,Rectangle,Ellipse,Line,Arrow,Freehand,Text,SpeechBalloon,Step,Image,Emoji,Cursor,Highlight,SmartEraser,Blur,Pixelate,Magnify,Spotlight,Crop,CutOut,Background,ImageEffects".to_string(),
        }
    }
}

#[derive(Debug)]
pub struct ShareXRequest {
    result_path: PathBuf,
    options_path: PathBuf,
    forwarded_to_existing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareXPrewarmOutcome {
    ExistingInstance,
    Spawned,
}

impl ShareXPrewarmOutcome {
    pub fn dispatch_label(self) -> &'static str {
        match self {
            Self::ExistingInstance => "existing_instance_pipe",
            Self::Spawned => "spawned",
        }
    }
}

impl ShareXRequest {
    pub fn result_path(&self) -> &Path {
        &self.result_path
    }

    pub fn dispatch_label(&self) -> &'static str {
        if self.forwarded_to_existing {
            "existing_instance_pipe"
        } else {
            "spawned"
        }
    }
}

impl Drop for ShareXRequest {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.result_path);
        let _ = fs::remove_file(&self.options_path);
    }
}

pub fn launch_region_capture(
    output_path: Option<&Path>,
    options: &ShareXCaptureOptions,
) -> Result<ShareXRequest, String> {
    launch_sharex("KaixinRectangleRegion", None, output_path, options)
}

pub fn launch_window_capture(
    hwnd: isize,
    output_path: Option<&Path>,
    options: &ShareXCaptureOptions,
) -> Result<ShareXRequest, String> {
    if hwnd == 0 {
        return Err("ShareX 指定窗口截图缺少有效窗口句柄。".to_string());
    }
    launch_sharex(
        "KaixinCaptureWindow",
        Some(hwnd.to_string()),
        output_path,
        options,
    )
}

/// Starts the bundled ShareX integration in the background without opening a
/// capture surface. Keeping this hidden instance warm avoids paying the .NET,
/// Avalonia and settings initialization cost on the first screenshot hotkey.
pub fn prewarm() -> Result<ShareXPrewarmOutcome, String> {
    let arguments = vec![
        "-portable".to_string(),
        "-silent".to_string(),
        "-KaixinPrewarm".to_string(),
    ];

    if forward_to_existing_instance(&arguments) {
        return Ok(ShareXPrewarmOutcome::ExistingInstance);
    }

    let Some(path) = resolve_sharex_path() else {
        return Err(
            "未找到随开心输入法安装的 ShareX。请重新运行安装程序或设置 KAIXIN_SHAREX_EXE。"
                .to_string(),
        );
    };

    Command::new(&path)
        .args(&arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ShareXPrewarmOutcome::Spawned)
        .map_err(|err| format!("无法预热 ShareX：{err}\n{}", path.display()))
}

/// Launch the Windows built-in clipping UI. This is the final compatibility
/// fallback when GPU capture backends are unavailable.
pub fn launch_system_screenclip() -> Result<(), String> {
    Command::new("explorer.exe")
        .arg("ms-screenclip:")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("无法启动 Windows 截图工具：{err}"))
}

pub fn resolve_sharex_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KAIXIN_SHAREX_EXE").map(PathBuf::from) {
        if is_sharex_exe(&path) {
            return Some(path);
        }
    }

    let current_exe = std::env::current_exe().ok()?;
    let current_dir = current_exe.parent()?;
    let packaged = current_dir.join("ShareX").join(SHAREX_EXE);
    if is_sharex_exe(&packaged) {
        return Some(packaged);
    }

    for ancestor in current_dir.ancestors().take(5) {
        let vendored_root = ancestor.join("third_party").join("ShareX");
        let published = vendored_root
            .join("publish")
            .join("win-x64")
            .join(SHAREX_EXE);
        if is_sharex_exe(&published) {
            return Some(published);
        }

        let source_build = vendored_root
            .join("ShareX")
            .join("bin")
            .join("Release")
            .join("win-x64")
            .join(SHAREX_EXE);
        if is_sharex_exe(&source_build) {
            return Some(source_build);
        }
        let source_build = vendored_root
            .join("ShareX")
            .join("bin")
            .join("Release")
            .join(SHAREX_EXE);
        if is_sharex_exe(&source_build) {
            return Some(source_build);
        }
    }

    None
}

fn launch_sharex(
    command: &str,
    parameter: Option<String>,
    output_path: Option<&Path>,
    options: &ShareXCaptureOptions,
) -> Result<ShareXRequest, String> {
    let result_path = create_result_path()?;
    let options_path = result_path.with_extension("options.json");
    let options_json =
        serde_json::to_vec(options).map_err(|err| format!("无法序列化 ShareX 截图设置：{err}"))?;
    if let Err(err) = fs::write(&options_path, options_json) {
        let _ = fs::remove_file(&options_path);
        return Err(format!(
            "无法写入 ShareX 截图设置 {}：{err}",
            options_path.display()
        ));
    }
    let mut arguments = vec![
        "-portable".to_string(),
        "-silent".to_string(),
        format!("-{command}"),
    ];
    if let Some(parameter) = parameter {
        arguments.push(parameter);
    }
    if let Some(output_path) = output_path {
        // ShareX writes the edited image to the exact timestamped path chosen
        // by Kaixin; ShareX's own filename pattern is never used here.
        arguments.push("-KaixinOutputPath".to_string());
        arguments.push(output_path.to_string_lossy().into_owned());
    }
    arguments.push("-KaixinOptionsPath".to_string());
    arguments.push(options_path.to_string_lossy().into_owned());
    arguments.push("-KaixinResultPath".to_string());
    arguments.push(result_path.to_string_lossy().into_owned());

    if forward_to_existing_instance(&arguments) {
        return Ok(ShareXRequest {
            result_path,
            options_path,
            forwarded_to_existing: true,
        });
    }

    let Some(path) = resolve_sharex_path() else {
        let _ = fs::remove_file(&options_path);
        return Err(
            "未找到随开心输入法安装的 ShareX。请重新运行安装程序或设置 KAIXIN_SHAREX_EXE。"
                .to_string(),
        );
    };

    let mut process = Command::new(&path);
    process
        .args(&arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match process.spawn() {
        Ok(_) => Ok(ShareXRequest {
            result_path,
            options_path,
            forwarded_to_existing: false,
        }),
        Err(err) => {
            let _ = fs::remove_file(&options_path);
            Err(format!("无法启动 ShareX：{err}\n{}", path.display()))
        }
    }
}

fn create_result_path() -> Result<PathBuf, String> {
    let local_appdata = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA 未设置，无法创建 ShareX 结果通道。".to_string())?;
    let root = local_appdata.join("kaixin").join("sharex-results");
    fs::create_dir_all(&root)
        .map_err(|err| format!("无法创建 ShareX 结果目录 {}：{err}", root.display()))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(root.join(format!(
        "{}-{timestamp}-{sequence}.result",
        std::process::id()
    )))
}

fn forward_to_existing_instance(arguments: &[String]) -> bool {
    let Some(computer_name) = std::env::var_os("COMPUTERNAME") else {
        return false;
    };
    let Some(user_name) = std::env::var_os("USERNAME") else {
        return false;
    };
    let pipe_path = format!(
        r"\\.\pipe\{}-{}-{SHAREX_PIPE_SUFFIX}",
        computer_name.to_string_lossy(),
        user_name.to_string_lossy()
    );
    let Ok(mut pipe) = OpenOptions::new().write(true).open(pipe_path) else {
        return false;
    };
    write_dotnet_arguments(&mut pipe, arguments).is_ok() && pipe.flush().is_ok()
}

fn write_dotnet_arguments<W: Write>(writer: &mut W, arguments: &[String]) -> std::io::Result<()> {
    writer.write_all(&(arguments.len() as i32).to_le_bytes())?;
    for argument in arguments {
        let bytes = argument.as_bytes();
        write_7_bit_encoded_length(writer, bytes.len())?;
        writer.write_all(bytes)?;
    }
    Ok(())
}

fn write_7_bit_encoded_length<W: Write>(writer: &mut W, mut value: usize) -> std::io::Result<()> {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        writer.write_all(&[byte])?;
        if value == 0 {
            return Ok(());
        }
    }
}

fn is_sharex_exe(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(SHAREX_EXE))
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_window_handle() {
        assert!(launch_window_capture(0, None, &ShareXCaptureOptions::default()).is_err());
    }

    #[test]
    fn serializes_versioned_capture_options_for_sharex() {
        let options = ShareXCaptureOptions {
            open_editor: false,
            jpeg_quality: 83,
            ..ShareXCaptureOptions::default()
        };
        let value = serde_json::to_value(options).unwrap();
        assert_eq!(value["Version"], 1);
        assert_eq!(value["OpenEditor"], false);
        assert_eq!(value["JpegQuality"], 83);
    }

    #[test]
    fn encodes_arguments_like_dotnet_binary_writer() {
        let mut encoded = Vec::new();
        write_dotnet_arguments(&mut encoded, &["-silent".to_string(), "截图".to_string()]).unwrap();
        assert_eq!(&encoded[..4], &2i32.to_le_bytes());
        assert_eq!(encoded[4], 7);
        assert_eq!(&encoded[5..12], b"-silent");
        assert_eq!(encoded[12], 6);
        assert_eq!(&encoded[13..], "截图".as_bytes());
    }
}
