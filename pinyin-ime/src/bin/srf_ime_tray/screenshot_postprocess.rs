fn finish_captured_screenshot(
    captured: CapturedScreenshot,
    action_target: HWND,
    prefs: &ScreenshotPrefs,
) -> Result<bool, String> {
    if captured.action == screenshot_region_selector::RegionAction::SaveAs {
        let Some(destination) = rfd::FileDialog::new()
            .set_title("另存截图")
            .add_filter("PNG 图片", &["png"])
            .add_filter("JPG 图片", &["jpg", "jpeg"])
            .set_file_name("截图.png")
            .save_file()
        else {
            return Ok(false);
        };
        let destination = if destination.extension().is_none() {
            destination.with_extension("png")
        } else {
            destination
        };
        let mut save_prefs = screenshot_prefs();
        save_prefs.format = match destination.extension().and_then(|value| value.to_str()) {
            Some(value)
                if value.eq_ignore_ascii_case("jpg") || value.eq_ignore_ascii_case("jpeg") =>
            {
                "jpg"
            }
            Some(value) if value.eq_ignore_ascii_case("png") => "png",
            _ => return Err("另存截图仅支持 PNG 或 JPG 文件名。".to_string()),
        }
        .to_string();
        return save_screenshot_image(&captured.image, &destination, true, &save_prefs)
            .map(|()| true);
    }
    if captured.action == screenshot_region_selector::RegionAction::Annotate {
        let destination = next_temporary_screenshot_path()?;
        let mut save_prefs = screenshot_prefs();
        save_prefs.format = "png".to_string();
        save_screenshot_image(&captured.image, &destination, false, &save_prefs)?;
        Command::new("mspaint.exe")
            .arg(&destination)
            .spawn()
            .map_err(|err| format!("打开画图标注失败：{err}"))?;
        return Ok(true);
    }
    if captured.action == screenshot_region_selector::RegionAction::Pin {
        pinned_screenshot::spawn(captured.image);
        return Ok(true);
    }
    let mut action_prefs = screenshot_prefs();
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

    if needs_handoff_path {
        if prefs.auto_save {
            let destination = next_screenshot_path(prefs, action_target);
            match destination.and_then(|destination| {
                let destination = if prefs.name_pattern.contains("{width}")
                    || prefs.name_pattern.contains("{height}")
                {
                    screenshot_path_with_dimensions(
                        &destination,
                        prefs,
                        action_target,
                        captured.image.width(),
                        captured.image.height(),
                    )?
                } else {
                    destination
                };
                save_screenshot_image(&captured.image, &destination, true, prefs)
                    .map(|()| destination)
            }) {
                Ok(destination) => {
                    persistent = true;
                    let _ = save_silent_screenshot_copy(&destination, prefs);
                    path = Some(destination);
                }
                Err(err) => errors.push(format!("保存截图失败：{err}")),
            }
        }
        if path.is_none() && (prefs.ocr_after_capture || prefs.translate_after_capture) {
            match next_temporary_screenshot_path().and_then(|destination| {
                save_screenshot_image(&captured.image, &destination, false, prefs)
                    .map(|()| destination)
            }) {
                Ok(destination) => path = Some(destination),
                Err(err) => errors.push(format!("创建 OCR 交接文件失败：{err}")),
            }
        }
    }

    if persistent {
        if let Some(saved_path) = path.as_deref() {
            record_last_screenshot(saved_path);
            record_screenshot_metadata_async(saved_path.to_path_buf(), captured.source);
        }
    }

    let copy_status = if !prefs.copy_after_capture {
        "disabled"
    } else if let Err(err) = copy_image_to_clipboard(&captured.image) {
        errors.push(err);
        "failed"
    } else {
        "ok"
    };

    if prefs.ocr_after_capture || prefs.translate_after_capture {
        if let Some(image_path) = path.as_deref() {
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "screenshot_ocr_handoff",
                format!(
                    "status=launch path={} target_hwnd={} translate={}",
                    runtime_log::path_for_log(image_path),
                    action_target as isize,
                    if prefs.translate_after_capture { 1 } else { 0 }
                ),
            );
            if let Err(err) = open_ocr_with_image(
                action_target,
                image_path,
                prefs.translate_after_capture,
                !persistent,
            ) {
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
    persistent: bool,
    prefs: &ScreenshotPrefs,
) -> Result<(), String> {
    let format = if persistent && prefs.format == "jpg" {
        image::ImageFormat::Jpeg
    } else {
        image::ImageFormat::Png
    };
    // Desktop capture surfaces are BGRA and some drivers leave their alpha
    // channel undefined.  PNG viewers then composite an otherwise correct
    // frame against their accent colour (often purple).  A screenshot is
    // an opaque desktop image, so flatten alpha before both file encoding
    // and clipboard publication.
    image::DynamicImage::ImageRgba8(opaque_screenshot_image(image))
        .save_with_format(path, format)
        .map_err(|err| format!("写入截图 {} 失败：{err}", path.display()))
}

fn copy_image_to_clipboard(image: &image::RgbaImage) -> Result<(), String> {
    let image = opaque_screenshot_image(image);
    let png = encode_clipboard_png(&image)?;
    let dibv5 = encode_clipboard_dibv5(&image)?;
    write_screenshot_formats_to_clipboard(&png, &dibv5)
}

fn encode_clipboard_png(image: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|err| format!("编码剪贴板 PNG 失败：{err}"))?;
    Ok(bytes.into_inner())
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
        for pixel in row.chunks_exact(4) {
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

fn save_silent_screenshot_copy(source: &Path, prefs: &ScreenshotPrefs) -> Option<PathBuf> {
    if !prefs.silent_copy_enabled {
        return None;
    }
    let Some(dir) = prefs.silent_copy_dir.as_ref() else {
        return None;
    };
    match copy_screenshot_file_to_dir(source, dir) {
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
            Some(path)
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
            None
        }
    }
}

fn copy_screenshot_file_to_dir(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let dest = unique_screenshot_copy_path(source, dir)?;
    fs::copy(source, &dest)
        .map(|_| dest.clone())
        .map_err(|e| format!("copy {} to {}: {e}", source.display(), dest.display()))
}

fn unique_screenshot_copy_path(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    let file_name = source
        .file_name()
        .ok_or_else(|| format!("source path has no file name: {}", source.display()))?;
    let mut path = dir.join(file_name);
    if !path.exists() {
        return Ok(path);
    }

    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("screenshot");
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty());
    for idx in 2..1000 {
        let file_name = if let Some(extension) = extension {
            format!("{stem}_{idx:03}.{extension}")
        } else {
            format!("{stem}_{idx:03}")
        };
        path = dir.join(file_name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Ok(path)
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
        target_hwnd: source.target_hwnd as isize,
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
