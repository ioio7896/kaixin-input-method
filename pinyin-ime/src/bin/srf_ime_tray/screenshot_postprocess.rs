use std::io::{self, BufWriter, Write};
use std::os::windows::ffi::OsStrExt;

fn handle_captured_screenshot(
    captured: CapturedScreenshot,
    target: HWND,
    prefs: ScreenshotPrefs,
    ocr_available: bool,
    one_shot: bool,
) {
    if prefs.show_result_window {
        match pinned_screenshot::show_result(
            captured.clone(),
            target,
            ocr_available,
            pinned_screenshot::InitialAction::Run(prefs.clone()),
            one_shot,
        ) {
            Ok(()) => return,
            Err(err) => runtime_log::log_tray(
                RuntimeLogLevel::Error,
                "screenshot_result_window",
                format!("status=failed reason={err}"),
            ),
        }
    }
    let (result, silent_copy) = process_captured_screenshot(&captured, target, &prefs);
    if matches!(result, Ok(true)) {
        if let Some(copy) = silent_copy {
            copy.run();
        }
        return;
    }
    let worker = silent_copy.and_then(|copy| thread::Builder::new().spawn(move || copy.run()).ok());
    if let Err(err) = pinned_screenshot::show_result(
        captured,
        target,
        ocr_available,
        pinned_screenshot::InitialAction::Completed(result),
        one_shot,
    ) {
        show_error_message(&format!("无法打开截图操作窗口：{err}"));
    }
    if one_shot {
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

struct SilentScreenshotCopy {
    source: PathBuf,
    dir: PathBuf,
    trace: ScreenshotTrace,
}

impl SilentScreenshotCopy {
    fn run(self) {
        save_silent_screenshot_copy(&self.source, &self.dir, self.trace);
    }
}

fn process_captured_screenshot(
    captured: &CapturedScreenshot,
    action_target: HWND,
    prefs: &ScreenshotPrefs,
) -> (Result<bool, String>, Option<SilentScreenshotCopy>) {
    let stage = captured.trace.stage("postprocess");
    let mut silent_copy = None;
    let result = finish_captured_screenshot(captured, action_target, prefs, &mut silent_copy);
    stage.finish(screenshot_action_status(&result));
    (result, silent_copy)
}

fn screenshot_action_status(result: &Result<bool, String>) -> &'static str {
    match result {
        Ok(true) => "ok",
        Ok(false) => "cancelled",
        Err(_) => "failed",
    }
}

fn finish_captured_screenshot(
    captured: &CapturedScreenshot,
    action_target: HWND,
    prefs: &ScreenshotPrefs,
    silent_copy: &mut Option<SilentScreenshotCopy>,
) -> Result<bool, String> {
    let trace = captured.trace;
    if captured.action == screenshot_region_selector::RegionAction::SaveAs {
        let stage = trace.stage("save_dialog");
        let destination = rfd::FileDialog::new()
            .set_title("另存截图")
            .add_filter("PNG 图片", &["png"])
            .add_filter("JPG 图片", &["jpg", "jpeg"])
            .set_file_name("截图.png")
            .save_file();
        stage.finish(if destination.is_some() {
            "ok"
        } else {
            "cancelled"
        });
        let Some(destination) = destination else {
            return Ok(false);
        };
        let destination = if destination.extension().is_none() {
            destination.with_extension("png")
        } else {
            destination
        };
        let format = match destination.extension().and_then(|value| value.to_str()) {
            Some(value)
                if value.eq_ignore_ascii_case("jpg") || value.eq_ignore_ascii_case("jpeg") =>
            {
                image::ImageFormat::Jpeg
            }
            Some(value) if value.eq_ignore_ascii_case("png") => image::ImageFormat::Png,
            _ => return Err("另存截图仅支持 PNG 或 JPG 文件名。".to_string()),
        };
        return save_screenshot_image(&captured.image, &destination, format, trace).map(|()| true);
    }
    if captured.action == screenshot_region_selector::RegionAction::Annotate {
        let destination = trace.measure("save_prepare", next_temporary_screenshot_path)?;
        save_screenshot_image(
            &captured.image,
            &destination,
            image::ImageFormat::Png,
            trace,
        )?;
        trace.measure("annotate_launch", || {
            Command::new("mspaint.exe")
                .arg(&destination)
                .spawn()
                .map_err(|err| format!("打开画图标注失败：{err}"))
        })?;
        return Ok(true);
    }
    if captured.action == screenshot_region_selector::RegionAction::Pin {
        pinned_screenshot::spawn(captured.image.clone(), trace);
        return Ok(true);
    }
    let mut action_prefs = prefs.clone();
    match captured.action {
        screenshot_region_selector::RegionAction::Copy => {
            action_prefs.auto_save = false;
            action_prefs.copy_after_capture = true;
            action_prefs.ocr_after_capture = false;
            action_prefs.translate_after_capture = false;
        }
        screenshot_region_selector::RegionAction::Ocr => {
            action_prefs.auto_save = false;
            action_prefs.copy_after_capture = false;
            action_prefs.ocr_after_capture = true;
            action_prefs.translate_after_capture = false;
        }
        _ => {}
    }
    let prefs = if captured.action == screenshot_region_selector::RegionAction::Configured {
        prefs
    } else {
        &action_prefs
    };
    let needs_handoff_path =
        prefs.auto_save || prefs.ocr_after_capture || prefs.translate_after_capture;
    let mut path = None;
    let mut persistent = false;
    let mut errors = Vec::new();

    let copy_status = if !prefs.copy_after_capture {
        "disabled"
    } else if let Err(err) = copy_image_to_clipboard(&captured.image, trace) {
        errors.push(err);
        "failed"
    } else {
        "ok"
    };

    if needs_handoff_path {
        if prefs.auto_save {
            match save_automatic_screenshot(&captured.image, prefs, action_target, trace) {
                Ok(destination) => {
                    persistent = true;
                    if let Some(dir) = prefs
                        .silent_copy_dir
                        .as_ref()
                        .filter(|_| prefs.silent_copy_enabled)
                    {
                        *silent_copy = Some(SilentScreenshotCopy {
                            source: destination.clone(),
                            dir: dir.clone(),
                            trace,
                        });
                    }
                    path = Some(destination);
                }
                Err(err) => errors.push(format!("保存截图失败：{err}")),
            }
        }
        if path.is_none() && (prefs.ocr_after_capture || prefs.translate_after_capture) {
            match trace
                .measure("save_prepare", next_temporary_screenshot_path)
                .and_then(|destination| {
                    save_screenshot_image(
                        &captured.image,
                        &destination,
                        image::ImageFormat::Png,
                        trace,
                    )
                    .map(|()| destination)
                }) {
                Ok(destination) => path = Some(destination),
                Err(err) => errors.push(format!("创建 OCR 交接文件失败：{err}")),
            }
        }
    }

    if persistent {
        if let Some(saved_path) = path.as_deref() {
            let stage = trace.stage("save_record");
            record_last_screenshot(saved_path);
            record_screenshot_metadata_async(saved_path.to_path_buf(), captured.source);
            stage.finish("ok");
        }
    }

    if prefs.ocr_after_capture || prefs.translate_after_capture {
        if let Some(image_path) = path.as_deref() {
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "screenshot_ocr_handoff",
                format!(
                    "status=launch path={} target_hwnd={} translate={}",
                    runtime_log::path_for_log(image_path),
                    { action_target },
                    if prefs.translate_after_capture { 1 } else { 0 }
                ),
            );
            if let Err(err) = trace.measure("ocr_handoff", || {
                open_ocr_with_image(
                    action_target,
                    image_path,
                    prefs.translate_after_capture,
                    !persistent,
                )
            }) {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "screenshot_ocr_handoff",
                    format!("status=failed reason={err}"),
                );
                if !persistent {
                    let _ = fs::remove_file(image_path);
                }
                errors.push(format!("OCR 启动失败：{err}"));
            }
        } else {
            errors.push("OCR 交接缺少可读取的截图文件。".to_string());
        }
    }

    let final_status = if errors.is_empty() { "ok" } else { "partial" };
    runtime_log::log_tray(
            if errors.is_empty() {
                RuntimeLogLevel::Basic
            } else {
                RuntimeLogLevel::Error
            },
            "screenshot_postprocess",
            format!(
                "status={} backend={} source={} path={} persistent={} copy={} ocr={} translate={} errors={}",
                final_status,
                captured.source.backend.as_config(),
                captured.source.source_key(),
                path.as_deref()
                    .map(runtime_log::path_for_log)
                    .unwrap_or_else(|| "none".to_string()),
                if persistent { 1 } else { 0 },
                copy_status,
                if prefs.ocr_after_capture { 1 } else { 0 },
                if prefs.translate_after_capture { 1 } else { 0 },
                errors.join(" | ")
            ),
        );
    if errors.is_empty() {
        Ok(true)
    } else {
        Err(errors.join("\n"))
    }
}

fn valid_saved_screenshot(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
}

fn save_screenshot_image(
    image: &image::RgbaImage,
    path: &Path,
    format: image::ImageFormat,
    trace: ScreenshotTrace,
) -> Result<(), String> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut pending = encode_screenshot_file(image, dir, format, trace)?;
    trace.measure("save_publish", || {
        pending
            .publish(path, true)
            .map_err(|err| format!("写入截图 {} 失败：{err}", path.display()))
    })
}

struct PendingScreenshotFile(PathBuf);

impl PendingScreenshotFile {
    fn create(dir: &Path) -> io::Result<(Self, fs::File)> {
        loop {
            let sequence = SCREENSHOT_HANDOFF_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!(
                ".kaixin-screenshot-{}-{sequence}.tmp",
                std::process::id()
            ));
            match fs::File::options().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self(path), file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn publish(&mut self, path: &Path, overwrite: bool) -> io::Result<()> {
        if overwrite {
            fs::rename(&self.0, path)?;
        } else {
            use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
            let wide_path = |path: &Path| {
                path.as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            let source = wide_path(&self.0);
            let destination = wide_path(path);
            if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        self.0.clear();
        Ok(())
    }
}

impl Drop for PendingScreenshotFile {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
        }
    }
}

fn encode_screenshot_file(
    image: &image::RgbaImage,
    dir: &Path,
    format: image::ImageFormat,
    trace: ScreenshotTrace,
) -> Result<PendingScreenshotFile, String> {
    trace.measure("save_encode_write", || {
        let (pending, output) = PendingScreenshotFile::create(dir)
            .map_err(|err| format!("创建截图临时文件失败：{err}"))?;
        let mut output = BufWriter::new(output);
        let encoded = match format {
            image::ImageFormat::Jpeg => {
                image::codecs::jpeg::JpegEncoder::new(&mut output).encode_image(image)
            }
            image::ImageFormat::Png => encode_screenshot_png(image, &mut output),
            _ => return Err("截图仅支持 PNG 和 JPG。".to_string()),
        };
        encoded.map_err(|err| format!("编码截图失败：{err}"))?;
        output
            .flush()
            .map_err(|err| format!("写入截图失败：{err}"))?;
        Ok(pending)
    })
}

fn copy_image_to_clipboard(image: &image::RgbaImage, trace: ScreenshotTrace) -> Result<(), String> {
    let (png, dibv5) = trace.measure("clipboard_encode", || {
        let mut png = Vec::new();
        encode_screenshot_png(image, &mut png)
            .map_err(|err| format!("编码剪贴板 PNG 失败：{err}"))?;
        Ok::<_, String>((png, encode_clipboard_dibv5(image)?))
    })?;
    trace.measure("clipboard_write", || {
        write_screenshot_formats_to_clipboard(&png, &dibv5)
    })
}

fn encode_screenshot_png(image: &image::RgbaImage, output: impl Write) -> image::ImageResult<()> {
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(output).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )
}

/// Builds the exact CF_DIBV5 payload expected by the Windows clipboard:
/// a 124-byte BITMAPV5HEADER followed by bottom-up BGRA pixels. Publishing
/// this directly avoids `clipboard-win::set_bitmap_with`, which only keeps
/// a 40-byte BITMAPINFOHEADER and corrupts the masks of RGBA V4 BMP files.
fn encode_clipboard_dibv5(image: &image::RgbaImage) -> Result<Vec<u8>, String> {
    const HEADER_SIZE: usize = 124;
    const BI_BITFIELDS: u32 = 3;
    const LCS_SRGB: u32 = 0x7352_4742;
    const LCS_GM_IMAGES: u32 = 4;

    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err("剪贴板图片尺寸不能为空。".to_string());
    }
    let width_i32 = i32::try_from(width).map_err(|_| "剪贴板图片宽度过大。")?;
    let height_i32 = i32::try_from(height).map_err(|_| "剪贴板图片高度过大。")?;
    let pixel_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "剪贴板图片缓冲区尺寸溢出。".to_string())?;
    let pixel_bytes_u32 =
        u32::try_from(pixel_bytes).map_err(|_| "剪贴板图片缓冲区超过 DIBV5 限制。".to_string())?;
    let capacity = HEADER_SIZE
        .checked_add(pixel_bytes as usize)
        .ok_or_else(|| "剪贴板 DIBV5 缓冲区尺寸溢出。".to_string())?;
    let mut dib = Vec::with_capacity(capacity);

    dib.extend_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    dib.extend_from_slice(&width_i32.to_le_bytes());
    dib.extend_from_slice(&height_i32.to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes()); // planes
    dib.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    dib.extend_from_slice(&BI_BITFIELDS.to_le_bytes());
    dib.extend_from_slice(&pixel_bytes_u32.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes()); // x pixels per metre
    dib.extend_from_slice(&0i32.to_le_bytes()); // y pixels per metre
    dib.extend_from_slice(&0u32.to_le_bytes()); // colours used
    dib.extend_from_slice(&0u32.to_le_bytes()); // important colours
    dib.extend_from_slice(&0x00ff_0000u32.to_le_bytes()); // red mask
    dib.extend_from_slice(&0x0000_ff00u32.to_le_bytes()); // green mask
    dib.extend_from_slice(&0x0000_00ffu32.to_le_bytes()); // blue mask
    dib.extend_from_slice(&0xff00_0000u32.to_le_bytes()); // alpha mask
    dib.extend_from_slice(&LCS_SRGB.to_le_bytes());
    dib.extend_from_slice(&[0u8; 36]); // CIEXYZTRIPLE endpoints
    dib.extend_from_slice(&0u32.to_le_bytes()); // gamma red
    dib.extend_from_slice(&0u32.to_le_bytes()); // gamma green
    dib.extend_from_slice(&0u32.to_le_bytes()); // gamma blue
    dib.extend_from_slice(&LCS_GM_IMAGES.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes()); // profile data offset
    dib.extend_from_slice(&0u32.to_le_bytes()); // profile size
    dib.extend_from_slice(&0u32.to_le_bytes()); // reserved
    debug_assert_eq!(dib.len(), HEADER_SIZE);

    let row_bytes = width as usize * 4;
    for row in image.as_raw().chunks_exact(row_bytes).rev() {
        for pixel in row.as_chunks::<4>().0 {
            dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    debug_assert_eq!(dib.len(), capacity);
    Ok(dib)
}

fn write_screenshot_formats_to_clipboard(png: &[u8], dibv5: &[u8]) -> Result<(), String> {
    const MAX_ATTEMPTS: u32 = 12;
    let png_format = clipboard_win::register_format("PNG")
        .map(|format| format.get())
        .ok_or_else(|| "注册 Windows PNG 剪贴板格式失败。".to_string())?;
    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        let result = (|| {
            let _clipboard = clipboard_win::Clipboard::new_attempts(4)
                .map_err(|err| format!("打开剪贴板失败：{err}"))?;
            clipboard_win::empty().map_err(|err| format!("清空剪贴板失败：{err}"))?;
            clipboard_win::raw::set_without_clear(png_format, png)
                .map_err(|err| format!("发布剪贴板 PNG 失败：{err}"))?;
            clipboard_win::raw::set_without_clear(clipboard_win::formats::CF_DIBV5, dibv5)
                .map_err(|err| format!("发布剪贴板 DIBV5 失败：{err}"))?;
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
        thread::sleep(Duration::from_millis(10 + u64::from(attempt) * 8));
    }
    Err(last_error.unwrap_or_else(|| "写入剪贴板失败：未知错误".to_string()))
}

fn opaque_screenshot_image(image: &image::RgbaImage) -> image::RgbaImage {
    let mut flattened = image.clone();
    for pixel in flattened.pixels_mut() {
        pixel[3] = 255;
    }
    flattened
}

/// The `clipboard-win` bitmap convenience setter deliberately preserves the
/// rest of the clipboard. That fails with ERROR_ALREADY_EXISTS (183) when
/// CF_BITMAP was already present. A screenshot is a complete image copy, so
/// replace the clipboard atomically after the DIB has been created instead.
fn next_temporary_screenshot_path() -> Result<PathBuf, String> {
    let dir = app_paths::local_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("screenshot-handoff");
    fs::create_dir_all(&dir)
        .map_err(|err| format!("创建截图临时目录 {} 失败：{err}", dir.display()))?;
    cleanup_old_temporary_screenshots(&dir);
    let stamp = Local::now().format("%Y%m%d_%H%M%S_%3f");
    let sequence = SCREENSHOT_HANDOFF_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!(
        "capture_{}_{}_{}.png",
        std::process::id(),
        stamp,
        sequence
    )))
}

fn cleanup_old_temporary_screenshots(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(24 * 60 * 60))
        .unwrap_or(std::time::UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map(|modified| modified < cutoff)
            .unwrap_or(false);
        if stale && path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

fn save_silent_screenshot_copy(source: &Path, dir: &Path, trace: ScreenshotTrace) {
    match trace.measure("silent_copy", || copy_screenshot_file_to_dir(source, dir)) {
        Ok(path) => {
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "screenshot_silent_copy",
                format!(
                    "status=saved source={} path={}",
                    runtime_log::path_for_log(source),
                    runtime_log::path_for_log(&path)
                ),
            );
        }
        Err(err) => {
            runtime_log::log_tray(
                RuntimeLogLevel::Error,
                "screenshot_silent_copy",
                format!(
                    "status=failed source={} dir={} reason={}",
                    runtime_log::path_for_log(source),
                    runtime_log::path_for_log(dir),
                    err
                ),
            );
        }
    }
}

fn copy_screenshot_file_to_dir(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file_name = source
        .file_name()
        .ok_or_else(|| format!("source path has no file name: {}", source.display()))?;
    let output_dir = fs::canonicalize(dir).map_err(|err| format!("读取静默保存目录失败：{err}"))?;
    let (mut pending, file) = PendingScreenshotFile::create(&output_dir)
        .map_err(|err| format!("创建静默保存临时文件失败：{err}"))?;
    drop(file);
    fs::copy(source, &pending.0)
        .map_err(|err| format!("复制截图 {} 失败：{err}", source.display()))?;
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("screenshot");
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty());
    for index in 1..=999 {
        let name = if index == 1 {
            file_name.to_os_string()
        } else if let Some(extension) = extension {
            format!("{stem}_{index:03}.{extension}").into()
        } else {
            format!("{stem}_{index:03}").into()
        };
        match pending.publish(&output_dir.join(&name), false) {
            Ok(()) => return Ok(dir.join(name)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(format!("发布静默保存文件失败：{err}")),
        }
    }
    Err("静默保存文件名冲突过多。".to_string())
}

fn last_screenshot_record_path() -> Option<PathBuf> {
    app_paths::local_data_dir().map(|dir| dir.join("last_screenshot.txt"))
}

fn record_last_screenshot(path: &Path) {
    let Some(record_path) = last_screenshot_record_path() else {
        return;
    };
    if let Some(dir) = record_path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(record_path, path.display().to_string());
}

fn record_screenshot_metadata_async(path: PathBuf, source: SavedScreenshotSource) {
    thread::spawn(move || {
        let record = screenshot_record_for_path(path, source);
        match screenshot_store::record_screenshot(&record) {
            Ok(()) => runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "screenshot_library",
                format!(
                    "status=recorded source={} path={}",
                    record.source,
                    runtime_log::path_for_log(&record.path)
                ),
            ),
            Err(err) => runtime_log::log_tray(
                RuntimeLogLevel::Error,
                "screenshot_library",
                format!(
                    "status=failed path={} reason={}",
                    runtime_log::path_for_log(&record.path),
                    err
                ),
            ),
        }
    });
}

fn screenshot_record_for_path(
    path: PathBuf,
    source: SavedScreenshotSource,
) -> screenshot_store::ScreenshotRecord {
    screenshot_store::ScreenshotRecord {
        path,
        source: source.source_key(),
        target_hwnd: source.target_hwnd,
        source_window_title: if source.kind == CapturedScreenshotKind::Window
            && source.target_hwnd != 0
        {
            Some(unsafe { window_title(source.target_hwnd) })
        } else {
            None
        },
        ..screenshot_store::ScreenshotRecord::default()
    }
}

fn last_screenshot_path() -> Option<PathBuf> {
    let record_path = last_screenshot_record_path()?;
    let text = fs::read_to_string(record_path).ok()?;
    let path = PathBuf::from(text.trim());
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}
