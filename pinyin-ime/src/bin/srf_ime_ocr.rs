#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../fonts.rs"]
mod fonts;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use eframe::egui::{
    self, Color32, ColorImage, ComboBox, Frame, Margin, RichText, Rounding, ScrollArea, Slider,
    Stroke, TextEdit, TextureHandle, TextureOptions, Vec2,
};
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};
use pinyin_ime::{
    app_paths, clipboard_store, dxgi_capture, external_translation, rapidocr_paths, runtime_log,
    screenshot_region_selector, screenshot_store, tool_prefs, win_paste, win_single_instance,
    windows_graphics_capture, windows_security,
};
use rusqlite::{params, Connection, DatabaseName};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::ptr::NonNull;
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    mpsc::{self, Receiver},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WINDOW_TITLE: &str = "开心输入法 OCR";
const OCR_SINGLE_INSTANCE_MUTEX: &str = "Local\\KaixinInput_Ocr_SingleInstance_v1";
const DEFAULT_WINDOW_SIZE: [f32; 2] = [900.0, 680.0];
const MIN_WINDOW_SIZE: [f32; 2] = [720.0, 520.0];
const LEFT_PANEL_MIN_WIDTH: f32 = 220.0;
const LEFT_PANEL_MAX_WIDTH: f32 = 420.0;
const RESULT_PANEL_TARGET_MIN_WIDTH: f32 = 320.0;
const RESULT_FRAME_EDGE_INSET: f32 = 2.0;
const RAPIDOCR_HOT_TIMEOUT: Duration = Duration::from_secs(30);
const RAPIDOCR_COLD_TIMEOUT: Duration = Duration::from_secs(90);
const RAPIDOCR_STDERR_LIMIT: usize = 16 * 1024;
const OPENCV_CROP_PROCESS_TIMEOUT: Duration = Duration::from_secs(12);
const OCR_WINDOW_HIDE_DELAY: Duration = Duration::from_millis(50);
const OCR_REQUEST_POLL_INTERVAL: Duration = Duration::from_millis(50);
const OCR_WINDOW_REQUEST_MAX_AGE_MS: u128 = 120_000;
const MAX_OCR_IMPORT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_OCR_IMPORT_PIXELS: u64 = 134_217_728;
const AUTO_CROP_MIN_CONFIDENCE: f32 = 0.70;
const HISTORY_LIMIT: usize = 30;
static OCR_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ACTIVE_RAPIDOCR_PID: AtomicU32 = AtomicU32::new(0);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OcrLanguageMode {
    ZhHans,
    English,
    Mixed,
    Auto,
}

impl OcrLanguageMode {
    const ALL: [Self; 4] = [Self::ZhHans, Self::English, Self::Mixed, Self::Auto];

    fn label(self) -> &'static str {
        match self {
            Self::ZhHans => "简体中文",
            Self::English => "英文",
            Self::Mixed => "中英混合",
            Self::Auto => "自动（中英比较）",
        }
    }

    fn rapidocr_lang(self) -> &'static str {
        match self {
            Self::ZhHans => "zh",
            Self::English => "en",
            Self::Mixed => "mixed",
            Self::Auto => "auto",
        }
    }

    fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "english" => Self::English,
            "mixed" => Self::Mixed,
            "auto" => Self::Auto,
            _ => Self::ZhHans,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OcrSpeedProfile {
    Fast,
    Balanced,
    Accurate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OcrExecutionProvider {
    Auto,
    Cpu,
    DirectMl,
    Cuda,
}

impl OcrExecutionProvider {
    const ALL: [Self; 4] = [Self::Auto, Self::Cpu, Self::DirectMl, Self::Cuda];

    fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "cpu" => Self::Cpu,
            "directml" | "dml" => Self::DirectMl,
            "cuda" => Self::Cuda,
            _ => Self::Auto,
        }
    }

    fn as_config(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::DirectMl => "directml",
            Self::Cuda => "cuda",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动",
            Self::Cpu => "CPU",
            Self::DirectMl => "DirectML",
            Self::Cuda => "CUDA",
        }
    }
}

impl OcrSpeedProfile {
    fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "fast" | "quick" => Self::Fast,
            "accurate" | "accuracy" | "high" => Self::Accurate,
            _ => Self::Balanced,
        }
    }

    fn as_config(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Accurate => "accurate",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Fast => "快速",
            Self::Balanced => "均衡",
            Self::Accurate => "高精度",
        }
    }

    fn max_side_len(self) -> u32 {
        match self {
            Self::Fast => 1280,
            Self::Balanced => 1920,
            Self::Accurate => 2560,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CropRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Default for CropRect {
    fn default() -> Self {
        Self {
            left: 0.0,
            top: 0.0,
            right: 1.0,
            bottom: 1.0,
        }
    }
}

impl CropRect {
    fn normalize(&mut self) {
        self.left = self.left.clamp(0.0, 0.98);
        self.top = self.top.clamp(0.0, 0.98);
        self.right = self.right.clamp(0.02, 1.0);
        self.bottom = self.bottom.clamp(0.02, 1.0);
        if self.right < self.left + 0.02 {
            self.right = (self.left + 0.02).min(1.0);
        }
        if self.bottom < self.top + 0.02 {
            self.bottom = (self.top + 0.02).min(1.0);
        }
    }

    fn is_full(self) -> bool {
        self.left <= 0.001 && self.top <= 0.001 && self.right >= 0.999 && self.bottom >= 0.999
    }
}

#[derive(Clone, Copy, Debug)]
struct OcrProcessOptions {
    profile: OcrSpeedProfile,
    language: OcrLanguageMode,
    crop: CropRect,
    auto_border_crop: bool,
    contrast: bool,
    binarize: bool,
    scale2x: bool,
    denoise: bool,
    invert: bool,
}

impl OcrProcessOptions {
    fn has_image_processing(self) -> bool {
        !self.crop.is_full()
            || self.auto_border_crop
            || self.contrast
            || self.binarize
            || self.scale2x
            || self.denoise
            || self.invert
    }

    fn has_post_crop_processing(self) -> bool {
        self.contrast || self.binarize || self.scale2x || self.denoise || self.invert
    }

    fn summary(self) -> String {
        let mut parts = vec![self.language.label().to_string()];
        if !self.crop.is_full() {
            parts.push("二次框选".to_string());
        }
        if self.auto_border_crop {
            parts.push("边框裁剪".to_string());
        }
        if self.contrast {
            parts.push("增强对比度".to_string());
        }
        if self.binarize {
            parts.push("黑白化".to_string());
        }
        if self.scale2x {
            parts.push("放大 2x".to_string());
        }
        if self.denoise {
            parts.push("去噪".to_string());
        }
        if self.invert {
            parts.push("反色".to_string());
        }
        parts.join(" / ")
    }
}

#[derive(Clone)]
struct OcrHistoryEntry {
    when: String,
    text: String,
}

#[derive(Clone, Debug, Default)]
struct OcrCaptureSaveNote {
    saved_path: Option<PathBuf>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureSource {
    Window,
    ManualRegion,
    FallbackRegion,
    ImageFile,
}

impl CaptureSource {
    fn label(self) -> &'static str {
        match self {
            Self::Window => "当前窗口截图",
            Self::ManualRegion => "手动框选",
            Self::FallbackRegion => "已改用手动框选",
            Self::ImageFile => "传入截图",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DetectedCrop {
    rect: CropRect,
    confidence: f32,
    applied: bool,
}

#[derive(Clone, Debug)]
struct CaptureSession {
    source: CaptureSource,
    target_hwnd: isize,
    original_path: PathBuf,
    input_path: Option<PathBuf>,
    fallback_reason: Option<String>,
    save_note: OcrCaptureSaveNote,
    detected_crop: Option<DetectedCrop>,
}

impl CaptureSession {
    fn source_label(&self) -> &'static str {
        self.source.label()
    }

    fn input_file_name(&self) -> Option<String> {
        self.input_path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
    }
}

struct CaptureResult {
    path: PathBuf,
    source: CaptureSource,
    fallback_reason: Option<String>,
}

struct OcrTextResult {
    text: String,
    lines: Vec<OcrReviewLine>,
    low_confidence_lines: Vec<OcrReviewLine>,
    engine: String,
    elapsed: Duration,
    options: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OcrTextFormat {
    Original,
    Continuous,
    Paragraphs,
    Table,
    Custom,
}

impl OcrTextFormat {
    const ALL: [Self; 4] = [
        Self::Original,
        Self::Continuous,
        Self::Paragraphs,
        Self::Table,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Original => "原始文本",
            Self::Continuous => "连续正文",
            Self::Paragraphs => "保留段落",
            Self::Table => "表格",
            Self::Custom => "手动编辑",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureMode {
    AutoWindow,
    ManualRegion,
}

fn capture_mode_label(mode: CaptureMode) -> &'static str {
    match mode {
        CaptureMode::AutoWindow => "auto_window",
        CaptureMode::ManualRegion => "manual_region",
    }
}

enum OcrWorkerMessage {
    Stage {
        generation: u64,
        status: &'static str,
    },
    Captured {
        generation: u64,
        session: CaptureSession,
    },
    CaptureFailed {
        generation: u64,
        mode: CaptureMode,
        error: String,
    },
    Finished {
        generation: u64,
        session: Option<CaptureSession>,
        processed_path: Option<PathBuf>,
        result: Result<OcrTextResult, String>,
    },
}

enum StartupCapture {
    Captured(CaptureSession),
    Failed(String),
    ManualRegion,
}

struct PreparedImage {
    path: PathBuf,
    owned_temp: bool,
    detected_crop: Option<DetectedCrop>,
    // The preview still uses `path`, but the persistent OCR worker can consume
    // this buffer directly and avoid decoding the same processed PNG again.
    transport_png: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OcrWindowRequest {
    created_ms: u128,
    target_hwnd: isize,
    #[serde(default)]
    manual_region: bool,
    translate: bool,
    image: Option<PathBuf>,
    #[serde(default)]
    delete_source_after_import: bool,
}

struct OcrApp {
    text: String,
    original_text: String,
    ocr_lines: Vec<OcrReviewLine>,
    text_format: OcrTextFormat,
    undo_state: Option<(String, OcrTextFormat)>,
    last_saved_text: String,
    review_lines: Vec<OcrReviewLine>,
    review_preview: Option<TextureHandle>,
    review_preview_error: Option<String>,
    selected_review: Option<usize>,
    status: String,
    target_hwnd: isize,
    receiver: Option<Receiver<OcrWorkerMessage>>,
    busy: bool,
    task_generation: u64,
    language: OcrLanguageMode,
    profile: OcrSpeedProfile,
    provider: OcrExecutionProvider,
    crop: CropRect,
    auto_border_crop: bool,
    contrast: bool,
    binarize: bool,
    scale2x: bool,
    denoise: bool,
    invert: bool,
    image_path: Option<PathBuf>,
    processed_path: Option<PathBuf>,
    preview: Option<TextureHandle>,
    preview_size: Option<[usize; 2]>,
    preview_error: Option<String>,
    preview_generation: u64,
    capture_session: Option<CaptureSession>,
    capture_save_note: OcrCaptureSaveNote,
    history: Vec<OcrHistoryEntry>,
    last_engine: String,
    translate_after_ocr: bool,
    show_history: bool,
    focus_result_once: bool,
    last_request_poll: Instant,
    pending_window_requests: VecDeque<OcrWindowRequest>,
    pending_window_capture_error: Option<String>,
}

impl OcrApp {
    fn begin_task(&mut self) -> u64 {
        self.review_lines.clear();
        self.review_preview = None;
        self.review_preview_error = None;
        self.selected_review = None;
        self.task_generation = self.task_generation.wrapping_add(1).max(1);
        self.busy = true;
        self.task_generation
    }

    fn cancel_task(&mut self) {
        if !self.busy {
            return;
        }
        self.task_generation = self.task_generation.wrapping_add(1).max(1);
        self.busy = false;
        self.receiver = None;
        self.status = "已取消当前 OCR 任务。后续排队请求仍会继续处理。".to_string();
        terminate_active_rapidocr_process();
    }

    fn new(
        cc: &eframe::CreationContext<'_>,
        target_hwnd: isize,
        translate_after_ocr: bool,
        startup_capture: StartupCapture,
    ) -> Self {
        let _ = fonts::install_cjk_fonts(&cc.egui_ctx);
        let mut app = Self {
            text: String::new(),
            original_text: String::new(),
            ocr_lines: Vec::new(),
            text_format: OcrTextFormat::Continuous,
            undo_state: None,
            last_saved_text: String::new(),
            status: "正在截取目标窗口...".to_string(),
            target_hwnd,
            receiver: None,
            busy: false,
            task_generation: 0,
            language: OcrLanguageMode::from_config(&tool_prefs::ocr_language()),
            profile: ocr_speed_profile(),
            provider: OcrExecutionProvider::from_config(&tool_prefs::ocr_execution_provider()),
            crop: CropRect::default(),
            auto_border_crop: false,
            contrast: false,
            binarize: false,
            scale2x: false,
            denoise: false,
            invert: false,
            image_path: None,
            processed_path: None,
            preview: None,
            review_lines: Vec::new(),
            review_preview: None,
            review_preview_error: None,
            selected_review: None,
            preview_size: None,
            preview_error: None,
            preview_generation: 0,
            capture_session: None,
            capture_save_note: OcrCaptureSaveNote::default(),
            history: load_ocr_history(),
            last_engine: String::new(),
            translate_after_ocr,
            show_history: false,
            focus_result_once: false,
            last_request_poll: Instant::now(),
            pending_window_requests: VecDeque::new(),
            pending_window_capture_error: None,
        };
        match startup_capture {
            StartupCapture::Captured(session) => {
                app.start_recognition_from_capture(&cc.egui_ctx, session)
            }
            StartupCapture::Failed(err) => {
                app.pending_window_capture_error = Some(err.clone());
                app.status = window_capture_failed_status(&err);
            }
            StartupCapture::ManualRegion => {
                app.status = "请选择要识别的屏幕区域。".to_string();
                app.start_capture(Some(&cc.egui_ctx), CaptureMode::ManualRegion);
            }
        }
        app
    }

    fn options(&self) -> OcrProcessOptions {
        OcrProcessOptions {
            profile: self.profile,
            language: self.language,
            crop: self.crop,
            auto_border_crop: self.auto_border_crop,
            contrast: self.contrast,
            binarize: self.binarize,
            scale2x: self.scale2x,
            denoise: self.denoise,
            invert: self.invert,
        }
    }

    fn poll_window_request(&mut self, ctx: &egui::Context) {
        if self.last_request_poll.elapsed() < OCR_REQUEST_POLL_INTERVAL {
            return;
        }
        self.last_request_poll = Instant::now();

        for _ in 0..32 {
            if let Some(request) = take_ocr_window_request() {
                runtime_log::log_ocr(
                    runtime_log::RuntimeLogLevel::Basic,
                    "ocr_window_request",
                    format!(
                        "status=queued busy={} target_hwnd={} manual_region={} translate={} image={}",
                        if self.busy { 1 } else { 0 },
                        request.target_hwnd,
                        if request.manual_region { 1 } else { 0 },
                        if request.translate { 1 } else { 0 },
                        request
                            .image
                            .as_deref()
                            .map(runtime_log::path_for_log)
                            .unwrap_or_else(|| "(none)".to_string())
                    ),
                );
                if self.busy {
                    self.status = "正在处理上一个 OCR 请求，新请求已排队。".to_string();
                }
                self.pending_window_requests.push_back(request);
            } else {
                break;
            }
        }

        if self.busy {
            return;
        }
        if let Some(request) = self.pending_window_requests.pop_front() {
            self.apply_window_request(ctx, request);
        }
    }

    fn apply_window_request(&mut self, ctx: &egui::Context, request: OcrWindowRequest) {
        runtime_log::log_ocr(
            runtime_log::RuntimeLogLevel::Basic,
            "ocr_window_request",
            format!(
                "target_hwnd={} manual_region={} translate={} image={}",
                request.target_hwnd,
                if request.manual_region { 1 } else { 0 },
                if request.translate { 1 } else { 0 },
                request
                    .image
                    .as_deref()
                    .map(runtime_log::path_for_log)
                    .unwrap_or_else(|| "(none)".to_string())
            ),
        );
        self.target_hwnd = request.target_hwnd;
        self.translate_after_ocr = request.translate;
        if let Some(image) = request.image {
            // A screenshot handoff does not go through `start_capture`, so an
            // already-running OCR window may still be hidden from a previous
            // capture. Reveal it before recognition starts so the user gets
            // the preview and progress immediately.
            self.reveal_window(ctx);
            match capture_session_from_image_file(
                &image,
                self.target_hwnd,
                request.delete_source_after_import,
            ) {
                Ok(session) => self.start_recognition_from_capture(ctx, session),
                Err(err) => self.status = format!("载入截图失败：{err}"),
            }
        } else if request.manual_region {
            self.start_capture(Some(ctx), CaptureMode::ManualRegion);
        } else {
            self.start_capture(Some(ctx), CaptureMode::AutoWindow);
        }
    }

    fn reveal_window(&self, ctx: &egui::Context) {
        // Capturing temporarily hides this window. Raising its window level before
        // showing it avoids Windows leaving the restored OCR window behind the
        // application that was just captured.
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn start_capture(&mut self, ctx: Option<&egui::Context>, mode: CaptureMode) {
        if self.busy {
            return;
        }
        let hide_before_capture = ctx.is_some();
        if let Some(ctx) = ctx {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.request_repaint_after(OCR_WINDOW_HIDE_DELAY);
        }
        self.crop = CropRect::default();
        self.capture_save_note = OcrCaptureSaveNote::default();
        let options = self.options();
        let target_hwnd = self.target_hwnd;
        let fallback_reason = if mode == CaptureMode::ManualRegion {
            self.pending_window_capture_error.take()
        } else {
            self.pending_window_capture_error = None;
            None
        };
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let generation = self.begin_task();
        self.status = if mode == CaptureMode::ManualRegion {
            if fallback_reason.is_some() {
                "窗口截图失败，已切换到配置的框选方案。请选择要识别的屏幕区域。".to_string()
            } else {
                "请选择要识别的屏幕区域。".to_string()
            }
        } else if target_hwnd == 0 {
            "没有可截取的目标窗口。".to_string()
        } else {
            "正在截取目标窗口...".to_string()
        };
        runtime_log::log_ocr(
            runtime_log::RuntimeLogLevel::Basic,
            "ocr_capture_start",
            format!(
                "mode={} target_hwnd={} has_fallback_reason={}",
                capture_mode_label(mode),
                target_hwnd,
                if fallback_reason.is_some() { 1 } else { 0 }
            ),
        );
        std::thread::spawn(move || {
            if hide_before_capture {
                std::thread::sleep(OCR_WINDOW_HIDE_DELAY);
            }
            let capture_started = Instant::now();
            match capture_screenshot_image(target_hwnd, mode) {
                Ok(capture) => {
                    show_ocr_window_best_effort();
                    runtime_log::log_ocr(
                        runtime_log::RuntimeLogLevel::Basic,
                        "ocr_capture_result",
                        format!(
                            "status=captured mode={} source={} target_hwnd={} elapsed_ms={} pixels={} path={}",
                            capture_mode_label(mode),
                            capture.source.label(),
                            target_hwnd,
                            capture_started.elapsed().as_millis(),
                            image::image_dimensions(&capture.path)
                                .map(|(width, height)| format!("{width}x{height}"))
                                .unwrap_or_else(|_| "unknown".to_string()),
                            runtime_log::path_for_log(&capture.path)
                        ),
                    );
                    let save_note = save_ocr_capture_if_enabled(&capture.path);
                    let (source, fallback_reason) = if let Some(reason) =
                        fallback_reason.filter(|value| {
                            mode == CaptureMode::ManualRegion && !value.trim().is_empty()
                        }) {
                        (CaptureSource::FallbackRegion, Some(reason))
                    } else {
                        (capture.source, capture.fallback_reason)
                    };
                    let mut session = CaptureSession {
                        source,
                        target_hwnd,
                        original_path: capture.path,
                        input_path: None,
                        fallback_reason,
                        save_note,
                        detected_crop: None,
                    };
                    let _ = tx.send(OcrWorkerMessage::Captured {
                        generation,
                        session: session.clone(),
                    });
                    let (processed_path, result, detected_crop) =
                        recognize_from_image(&session.original_path, options, |status| {
                            let _ = tx.send(OcrWorkerMessage::Stage { generation, status });
                        });
                    session.detected_crop = detected_crop;
                    let cleanup_path = processed_path.clone();
                    if tx
                        .send(OcrWorkerMessage::Finished {
                            generation,
                            session: Some(session),
                            processed_path,
                            result,
                        })
                        .is_err()
                    {
                        if let Some(path) = cleanup_path {
                            let _ = fs::remove_file(path);
                        }
                    }
                }
                Err(err) => {
                    show_ocr_window_best_effort();
                    runtime_log::log_ocr(
                        runtime_log::RuntimeLogLevel::Error,
                        "ocr_capture_result",
                        format!(
                            "status=failed mode={} target_hwnd={} reason={}",
                            capture_mode_label(mode),
                            target_hwnd,
                            err
                        ),
                    );
                    let _ = tx.send(OcrWorkerMessage::CaptureFailed {
                        generation,
                        mode,
                        error: err,
                    });
                }
            }
        });
    }

    fn start_recognition_from_capture(&mut self, ctx: &egui::Context, mut session: CaptureSession) {
        if self.busy {
            return;
        }
        self.capture_save_note = session.save_note.clone();
        self.crop = CropRect::default();
        let options = self.options();
        let path_for_worker = session.original_path.clone();
        self.replace_capture_session(ctx, session.clone());
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let generation = self.begin_task();
        self.set_capture_status();
        std::thread::spawn(move || {
            let (processed_path, result, detected_crop) =
                recognize_from_image(&path_for_worker, options, |status| {
                    let _ = tx.send(OcrWorkerMessage::Stage { generation, status });
                });
            session.detected_crop = detected_crop;
            let cleanup_path = processed_path.clone();
            if tx
                .send(OcrWorkerMessage::Finished {
                    generation,
                    session: Some(session),
                    processed_path,
                    result,
                })
                .is_err()
            {
                if let Some(path) = cleanup_path {
                    let _ = fs::remove_file(path);
                }
            }
        });
    }

    fn start_recognition(&mut self) {
        if self.busy {
            return;
        }
        let Some(path) = self.image_path.clone() else {
            self.status = "请先截图，再重新识别。".to_string();
            return;
        };
        self.crop.normalize();
        let options = self.options();
        let mut session = self.capture_session.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let generation = self.begin_task();
        self.status = "正在重新识别当前截图...".to_string();
        std::thread::spawn(move || {
            let (processed_path, result, detected_crop) =
                recognize_from_image(&path, options, |status| {
                    let _ = tx.send(OcrWorkerMessage::Stage { generation, status });
                });
            if let Some(session) = session.as_mut() {
                if detected_crop.is_some() || options.auto_border_crop {
                    session.detected_crop = detected_crop;
                }
            }
            let cleanup_path = processed_path.clone();
            if tx
                .send(OcrWorkerMessage::Finished {
                    generation,
                    session,
                    processed_path,
                    result,
                })
                .is_err()
            {
                if let Some(path) = cleanup_path {
                    let _ = fs::remove_file(path);
                }
            }
        });
    }

    fn poll_result(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.receiver.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(message) => match message {
                OcrWorkerMessage::Stage { generation, status } => {
                    if generation == self.task_generation {
                        self.status = status.to_string();
                        self.receiver = Some(rx);
                    }
                }
                OcrWorkerMessage::Captured {
                    generation,
                    session,
                } => {
                    if generation != self.task_generation {
                        return;
                    }
                    self.reveal_window(ctx);
                    self.replace_capture_session(ctx, session);
                    self.set_capture_status();
                    self.busy = true;
                    self.receiver = Some(rx);
                }
                OcrWorkerMessage::CaptureFailed {
                    generation,
                    mode,
                    error,
                } => {
                    if generation != self.task_generation {
                        return;
                    }
                    self.reveal_window(ctx);
                    self.busy = false;
                    self.receiver = None;
                    self.replace_processed_image(None);
                    if mode == CaptureMode::AutoWindow {
                        self.pending_window_capture_error = Some(error.clone());
                        self.status = window_capture_failed_status(&error);
                    } else {
                        self.status = format!("截图失败：{error}");
                    }
                }
                OcrWorkerMessage::Finished {
                    generation,
                    session,
                    processed_path,
                    result,
                } => {
                    if generation != self.task_generation {
                        if let Some(path) = processed_path {
                            let _ = std::fs::remove_file(path);
                        }
                        return;
                    }
                    self.reveal_window(ctx);
                    self.busy = false;
                    self.receiver = None;
                    if let Some(session) = session {
                        self.capture_save_note = session.save_note.clone();
                        self.capture_session = Some(session);
                    }
                    self.replace_processed_image(processed_path);
                    match result {
                        Ok(result) => self.apply_text_result(result),
                        Err(err) => {
                            self.status = format!("OCR 失败：{err}{}", self.failure_hint());
                        }
                    }
                }
            },
            Err(mpsc::TryRecvError::Empty) => {
                self.receiver = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.reveal_window(ctx);
                self.busy = false;
                self.receiver = None;
                self.status = "OCR 任务已中断。".to_string();
            }
        }
    }

    fn apply_text_result(&mut self, result: OcrTextResult) {
        self.review_lines = result.low_confidence_lines;
        self.review_preview = None;
        self.review_preview_error = None;
        self.selected_review = None;
        self.original_text = result.text.trim().to_string();
        self.ocr_lines = result.lines;
        self.undo_state = None;
        if self.text_format == OcrTextFormat::Custom {
            self.text_format = OcrTextFormat::Continuous;
        }
        self.text = self.formatted_original(self.text_format);
        self.last_saved_text.clear();
        self.last_engine = result.engine;
        self.update_screenshot_ocr_index();
        if self.text.is_empty() {
            self.status = if self.review_lines.is_empty() {
                format!("没有识别到文字。{}{}", result.options, self.failure_hint())
            } else {
                format!(
                    "发现 {} 行疑似文字，请展开低置信度校对。",
                    self.review_lines.len()
                )
            };
            self.focus_result_once = true;
            return;
        }

        self.focus_result_once = true;
        let count = self.text.chars().count();
        let history_status = match save_ocr_history(&self.text) {
            Ok(()) => {
                self.history = load_ocr_history();
                self.last_saved_text = self.text.trim().to_string();
                "已加入 OCR 历史和剪贴板历史"
            }
            Err(_) => "历史保存失败",
        };
        self.status = format!(
            "{}已识别 {count} 个字符，{} ms，{}，{}。",
            self.capture_session
                .as_ref()
                .map(|session| format!("{}，", session.source_label()))
                .unwrap_or_default(),
            result.elapsed.as_millis(),
            self.last_engine,
            history_status
        );
        if self.translate_after_ocr {
            match open_translate_from_ocr(
                &self.text,
                self.target_hwnd,
                self.screenshot_library_path().as_deref(),
            ) {
                Ok(()) => self.status.push_str(" 已发送到 WinTranslator。"),
                Err(err) => self
                    .status
                    .push_str(&format!(" 打开 WinTranslator 失败：{err}")),
            }
        }
    }

    fn formatted_original(&self, format: OcrTextFormat) -> String {
        match format {
            OcrTextFormat::Original => self.original_text.clone(),
            OcrTextFormat::Continuous => remove_line_breaks_smart(&self.original_text),
            OcrTextFormat::Paragraphs => keep_paragraphs_smart(&self.original_text),
            OcrTextFormat::Table => table_mode_with_positions(&self.original_text, &self.ocr_lines),
            OcrTextFormat::Custom => self.text.clone(),
        }
    }

    fn apply_text_format(&mut self, format: OcrTextFormat) {
        if self.text_format == format || self.original_text.is_empty() {
            return;
        }
        self.undo_state = Some((self.text.clone(), self.text_format));
        self.text_format = format;
        self.text = self.formatted_original(format);
        self.focus_result_once = true;
        self.status = if format == OcrTextFormat::Table && !self.text.contains('\t') {
            "未检测到可靠的表格列；已保留原始行内容。".to_string()
        } else {
            format!("已按“{}”整理，可撤销或恢复原文。", format.label())
        };
    }

    fn apply_text_transform(&mut self, transformed: String, status: &str) {
        if transformed == self.text {
            self.status = "当前文本无需调整。".to_string();
            return;
        }
        self.undo_state = Some((
            std::mem::replace(&mut self.text, transformed),
            self.text_format,
        ));
        self.text_format = OcrTextFormat::Custom;
        self.focus_result_once = true;
        self.status = status.to_string();
    }

    fn screenshot_library_path(&self) -> Option<PathBuf> {
        self.capture_session
            .as_ref()
            .and_then(|session| session.save_note.saved_path.clone())
    }

    fn update_screenshot_ocr_index(&self) {
        let Some(path) = self.screenshot_library_path() else {
            return;
        };
        match screenshot_store::update_ocr_text(&path, &self.text) {
            Ok(()) => runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Basic,
                "screenshot_library",
                format!(
                    "status=ocr_updated path={}",
                    runtime_log::path_for_log(&path)
                ),
            ),
            Err(err) => runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Error,
                "screenshot_library",
                format!(
                    "status=ocr_update_failed path={} reason={}",
                    runtime_log::path_for_log(&path),
                    err
                ),
            ),
        }
    }

    fn failure_hint(&self) -> String {
        if self
            .capture_session
            .as_ref()
            .and_then(|session| session.detected_crop)
            .is_some()
            && self.auto_border_crop
        {
            return " 可点“忽略裁剪重识别”，或改用“手动框选”。".to_string();
        }
        match self.capture_session.as_ref().map(|session| session.source) {
            Some(CaptureSource::Window) => " 可试试“手动框选”。".to_string(),
            Some(CaptureSource::FallbackRegion) | Some(CaptureSource::ManualRegion) => {
                " 可调整二次框选或增强对比度后重试。".to_string()
            }
            Some(CaptureSource::ImageFile) => " 可调整二次框选或关闭自动裁剪后重试。".to_string(),
            None => String::new(),
        }
    }

    fn replace_capture_session(&mut self, ctx: &egui::Context, session: CaptureSession) {
        self.cleanup_current_images();
        self.capture_save_note = session.save_note.clone();
        self.image_path = Some(session.original_path.clone());
        self.capture_session = Some(session);
        self.load_preview_texture(ctx);
    }

    fn set_capture_status(&mut self) {
        self.status = ocr_capture_status(self.capture_session.as_ref());
        if let Some(error) = self.preview_error.as_deref() {
            self.status.push_str(&format!(" 截图预览加载失败：{error}"));
        }
    }

    fn replace_processed_image(&mut self, path: Option<PathBuf>) {
        if let Some(old) = self.processed_path.take() {
            let _ = std::fs::remove_file(old);
        }
        self.processed_path = path;
    }

    fn load_preview_texture(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.image_path else {
            self.preview = None;
            self.preview_size = None;
            self.preview_error = None;
            return;
        };
        match load_color_image(path) {
            Ok((image, size)) => {
                self.preview_generation = self.preview_generation.wrapping_add(1);
                let name = format!("ocr_preview_{}", self.preview_generation);
                self.preview = Some(ctx.load_texture(name, image, TextureOptions::LINEAR));
                self.preview_size = Some(size);
                self.preview_error = None;
            }
            Err(err) => {
                self.preview = None;
                self.preview_size = None;
                self.preview_error = Some(err.clone());
                self.status = format!("截图预览加载失败：{err}");
            }
        }
    }

    fn copy_text(&mut self) {
        if self.text.trim().is_empty() {
            self.status = "没有可复制的识别文本。".to_string();
            return;
        }
        match copy_to_system_clipboard(&self.text) {
            Ok(()) => self.status = "已复制到剪贴板。".to_string(),
            Err(err) => self.status = format!("复制失败：{err}"),
        }
    }

    fn paste_text(&mut self, ctx: &egui::Context) {
        if self.text.trim().is_empty() {
            self.status = "没有可粘贴的识别文本。".to_string();
            return;
        }
        match copy_to_system_clipboard(&self.text) {
            Ok(()) => match win_paste::send_ctrl_v_to_target(self.target_hwnd) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(err) => self.status = format!("已复制，但快粘失败：{err}"),
            },
            Err(err) => self.status = format!("粘贴失败：{err}"),
        }
    }

    fn save_history_now(&mut self) {
        if self.text.trim().is_empty() {
            self.status = "没有可加入历史的识别文本。".to_string();
            return;
        }
        match save_ocr_history(&self.text) {
            Ok(()) => {
                self.history = load_ocr_history();
                self.last_saved_text = self.text.trim().to_string();
                self.status = "修改后的结果已保存到 OCR 历史和剪贴板历史。".to_string();
            }
            Err(err) => self.status = format!("保存历史失败：{err}"),
        }
    }

    fn translate_text(&mut self) {
        if self.text.trim().is_empty() {
            self.status = "没有可翻译的识别文本。".to_string();
            return;
        }
        match open_translate_from_ocr(
            &self.text,
            self.target_hwnd,
            self.screenshot_library_path().as_deref(),
        ) {
            Ok(()) => self.status = "已发送到 WinTranslator。".to_string(),
            Err(err) => self.status = format!("打开 WinTranslator 失败：{err}"),
        }
    }

    fn open_saved_capture_folder(&mut self) {
        let Some(path) = self.capture_save_note.saved_path.as_ref() else {
            self.status = "当前 OCR 截图没有自动保存文件。".to_string();
            return;
        };
        let folder = path.parent().unwrap_or(path);
        match open_folder(folder) {
            Ok(()) => self.status = "已打开截图保存文件夹。".to_string(),
            Err(err) => self.status = format!("打开文件夹失败：{err}"),
        }
    }

    fn reset_processing(&mut self) {
        self.crop = CropRect::default();
        self.auto_border_crop = true;
        self.contrast = false;
        self.binarize = false;
        self.scale2x = false;
        self.denoise = false;
        self.invert = false;
    }

    fn has_text(&self) -> bool {
        !self.text.trim().is_empty()
    }

    fn result_summary(&self) -> String {
        if self.busy {
            return "正在使用本地 OCR 识别".to_string();
        }
        if self.text.trim().is_empty() {
            return "等待识别结果".to_string();
        }
        let chars = self.text.chars().count();
        let lines = self
            .text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        if self.last_engine.trim().is_empty() {
            format!("{chars} 字 / {lines} 行")
        } else {
            format!("{} · {chars} 字 / {lines} 行", self.last_engine)
        }
    }

    fn cleanup_current_images(&mut self) {
        if let Some(path) = self.image_path.take() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = self.processed_path.take() {
            let _ = std::fs::remove_file(path);
        }
        self.preview = None;
        self.preview_size = None;
        self.preview_error = None;
        self.capture_session = None;
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("OCR 识别");
            ui.add_space(8.0);
            if ui
                .add_enabled(!self.busy, egui::Button::new("当前窗口截图"))
                .clicked()
            {
                self.start_capture(Some(ctx), CaptureMode::AutoWindow);
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("手动框选"))
                .clicked()
            {
                self.start_capture(Some(ctx), CaptureMode::ManualRegion);
            }
            if ui
                .add_enabled(
                    !self.busy && self.image_path.is_some(),
                    egui::Button::new("重新识别"),
                )
                .clicked()
            {
                self.start_recognition();
            }
            if ui
                .add_enabled(self.busy, egui::Button::new("取消"))
                .clicked()
            {
                self.cancel_task();
            }
            ui.separator();
            let has_text = self.has_text();
            let primary = egui::Button::new(RichText::new("复制文字").color(Color32::WHITE))
                .fill(Color32::from_rgb(37, 99, 235));
            if ui.add_enabled(!self.busy && has_text, primary).clicked() {
                self.copy_text();
            }
            if ui
                .add_enabled(!self.busy && has_text, egui::Button::new("粘贴回原窗口"))
                .clicked()
            {
                self.paste_text(ctx);
            }
            if ui
                .add_enabled(!self.busy && has_text, egui::Button::new("翻译"))
                .on_hover_text("使用 WinTranslator 主界面翻译识别文本")
                .clicked()
            {
                self.translate_text();
            }
        });
    }

    fn draw_left_panel(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("截图预览").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.capture_save_note.saved_path.is_some()
                        && ui.small_button("打开文件夹").clicked()
                    {
                        self.open_saved_capture_folder();
                    }
                    let received_image = self
                        .capture_session
                        .as_ref()
                        .is_some_and(|session| session.input_path.is_some());
                    let (label, color) = if received_image && self.busy {
                        ("已接收 · 自动识别中", Color32::from_rgb(37, 99, 235))
                    } else if received_image {
                        ("已接收", Color32::from_rgb(22, 163, 74))
                    } else if self.capture_save_note.error.is_some() {
                        ("保存失败", Color32::from_rgb(220, 38, 38))
                    } else if self.capture_save_note.saved_path.is_some() {
                        ("已保存", Color32::from_rgb(22, 163, 74))
                    } else {
                        ("未保存", Color32::from_rgb(100, 116, 139))
                    };
                    ui.label(RichText::new(label).small().color(color));
                });
            });

            if let Some(session) = self.capture_session.as_ref() {
                if let Some(file_name) = session.input_file_name() {
                    ui.label(
                        RichText::new(format!("来源文件：{file_name}"))
                            .small()
                            .color(Color32::from_rgb(71, 85, 105)),
                    )
                    .on_hover_text(
                        session
                            .input_path
                            .as_deref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_default(),
                    );
                }
            }

            let preview_height = (ui.available_height() * 0.32).clamp(148.0, 240.0);
            Frame::none()
                .fill(Color32::WHITE)
                .stroke(Stroke::new(1.0, Color32::from_rgb(226, 232, 240)))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(8.0, 8.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.set_min_height(preview_height);
                    if let Some(texture) = &self.preview {
                        // Reserve the complete preview canvas first. Letting Image choose its
                        // widget size from the source bitmap made very wide captures collapse
                        // into a tiny strip at the top-left of the panel.
                        let canvas_size =
                            Vec2::new(ui.available_width(), (preview_height - 48.0).max(88.0));
                        let (canvas_rect, _) =
                            ui.allocate_exact_size(canvas_size, egui::Sense::hover());
                        let source_size = texture.size_vec2();
                        let scale = (canvas_rect.width() / source_size.x.max(1.0))
                            .min(canvas_rect.height() / source_size.y.max(1.0));
                        let fitted_size = source_size * scale.max(0.01);
                        let image_rect =
                            egui::Rect::from_center_size(canvas_rect.center(), fitted_size);
                        ui.painter().image(
                            texture.id(),
                            image_rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        self.paint_detected_crop(ui, image_rect);
                        if let Some([w, h]) = self.preview_size {
                            ui.label(
                                RichText::new(format!("{w} x {h}"))
                                    .small()
                                    .color(Color32::from_rgb(100, 116, 139)),
                            );
                        }
                        self.draw_detected_crop_controls(ui);
                    } else {
                        ui.add_space(72.0);
                        ui.centered_and_justified(|ui| {
                            if let Some(error) = self.preview_error.as_deref() {
                                ui.label(
                                    RichText::new(format!("截图预览加载失败\n{error}"))
                                        .color(Color32::from_rgb(220, 38, 38)),
                                );
                            } else {
                                ui.label(
                                    RichText::new("暂无截图")
                                        .color(Color32::from_rgb(100, 116, 139)),
                                );
                            }
                        });
                        ui.add_space(72.0);
                    }
                });

            ui.add_space(10.0);
            ui.label(RichText::new("语言").strong());
            ComboBox::from_id_salt("ocr_language_mode")
                .selected_text(self.language.label())
                .show_ui(ui, |ui| {
                    for mode in OcrLanguageMode::ALL {
                        if ui
                            .selectable_value(&mut self.language, mode, mode.label())
                            .clicked()
                        {
                            let _ = tool_prefs::set_ocr_preference(
                                "language",
                                self.language.rapidocr_lang(),
                            );
                        }
                    }
                });

            ui.add_space(8.0);
            ui.label(RichText::new("识别速度").strong());
            ui.horizontal_wrapped(|ui| {
                for profile in [
                    OcrSpeedProfile::Fast,
                    OcrSpeedProfile::Balanced,
                    OcrSpeedProfile::Accurate,
                ] {
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::SelectableLabel::new(self.profile == profile, profile.label()),
                        )
                        .on_hover_text(match profile {
                            OcrSpeedProfile::Fast => "最长边 1280；优先使用可选轻量检测模型。",
                            OcrSpeedProfile::Balanced => "最长边 1920；适合多数屏幕文字。",
                            OcrSpeedProfile::Accurate => {
                                "最长边 2560；可在高级设置启用自动边框裁剪。"
                            }
                        })
                        .clicked()
                    {
                        self.profile = profile;
                        let _ = tool_prefs::set_ocr_preference("profile", profile.as_config());
                        if profile != OcrSpeedProfile::Accurate {
                            self.auto_border_crop = false;
                        }
                    }
                }
            });

            ui.add_space(10.0);
            egui::CollapsingHeader::new("校正 / 高级")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!self.busy, |ui| {
                        ui.label(RichText::new("图像处理").strong());
                        ui.add_space(8.0);
                        ui.label(RichText::new("推理后端").strong());
                        ui.horizontal_wrapped(|ui| {
                            for provider in OcrExecutionProvider::ALL {
                                if ui
                                    .selectable_value(
                                        &mut self.provider,
                                        provider,
                                        provider.label(),
                                    )
                                    .on_hover_text(
                                        "自动优先使用已安装的 CUDA/DirectML，否则回退 CPU。",
                                    )
                                    .clicked()
                                {
                                    let _ = tool_prefs::set_ocr_preference(
                                        "provider",
                                        provider.as_config(),
                                    );
                                    // Provider selection is fixed when ORT sessions are built.
                                    shutdown_rapidocr_process();
                                }
                            }
                        });
                        ui.checkbox(&mut self.auto_border_crop, "自动按边框裁剪");
                        ui.checkbox(&mut self.contrast, "增强对比度");
                        ui.checkbox(&mut self.binarize, "黑白化");
                        ui.checkbox(&mut self.scale2x, "放大 2x");
                        ui.checkbox(&mut self.denoise, "去噪");
                        ui.checkbox(&mut self.invert, "反色识别");

                        ui.add_space(10.0);
                        ui.label(RichText::new("二次框选").strong());
                        ui.add(Slider::new(&mut self.crop.left, 0.0..=0.95).text("左"));
                        ui.add(Slider::new(&mut self.crop.top, 0.0..=0.95).text("上"));
                        ui.add(Slider::new(&mut self.crop.right, 0.05..=1.0).text("右"));
                        ui.add(Slider::new(&mut self.crop.bottom, 0.05..=1.0).text("下"));
                        self.crop.normalize();

                        if ui.button("重置处理").clicked() {
                            self.reset_processing();
                        }
                    });
                });
        });
    }

    fn detected_crop(&self) -> Option<DetectedCrop> {
        self.capture_session
            .as_ref()
            .and_then(|session| session.detected_crop)
    }

    fn paint_detected_crop(&self, ui: &egui::Ui, image_rect: egui::Rect) {
        let Some(crop) = self.detected_crop() else {
            return;
        };
        let rect = crop.rect;
        let left = image_rect.left() + image_rect.width() * rect.left;
        let top = image_rect.top() + image_rect.height() * rect.top;
        let right = image_rect.left() + image_rect.width() * rect.right;
        let bottom = image_rect.top() + image_rect.height() * rect.bottom;
        let crop_rect = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom));
        ui.painter().rect_stroke(
            crop_rect,
            Rounding::same(2.0),
            Stroke::new(2.0, Color32::from_rgb(37, 99, 235)),
        );
    }

    fn draw_detected_crop_controls(&mut self, ui: &mut egui::Ui) {
        let Some(crop) = self.detected_crop() else {
            return;
        };
        ui.add_space(4.0);
        let label = if crop.applied {
            format!("已自动裁边，置信度 {:.3}", crop.confidence)
        } else {
            format!("检测到边框，但置信度 {:.3} 较低，未裁剪", crop.confidence)
        };
        ui.label(
            RichText::new(label)
                .small()
                .color(Color32::from_rgb(37, 99, 235)),
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.busy && self.auto_border_crop,
                    egui::Button::new("忽略裁剪重识别"),
                )
                .clicked()
            {
                self.auto_border_crop = false;
                self.start_recognition();
            }
            if ui
                .add_enabled(
                    !self.busy && !self.auto_border_crop,
                    egui::Button::new("使用裁剪重识别"),
                )
                .clicked()
            {
                self.auto_border_crop = self.profile == OcrSpeedProfile::Accurate;
                self.start_recognition();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("手动调整"))
                .clicked()
            {
                self.auto_border_crop = false;
                self.crop = crop.rect;
                self.crop.normalize();
                self.status = "已把检测框填入二次框选，可微调后重新识别。".to_string();
            }
        });
    }

    fn draw_review_panel(&mut self, ui: &mut egui::Ui) {
        if self.busy || self.review_lines.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(format!("低置信度校对（{} 行）", self.review_lines.len()))
            .id_source("ocr_low_confidence")
            .show(ui, |ui| {
                ui.label("选择疑似文字查看对应图像，修改后可加入正文。");
                egui::ScrollArea::vertical()
                    .id_source("ocr_review_lines")
                    .max_height(110.0)
                    .show(ui, |ui| {
                        for (index, line) in self.review_lines.iter().enumerate() {
                            if ui
                                .selectable_label(
                                    self.selected_review == Some(index),
                                    format!("{:.0}%  {}", line.score * 100.0, line.text),
                                )
                                .clicked()
                            {
                                self.selected_review = Some(index);
                                self.review_preview = None;
                                self.review_preview_error = None;
                            }
                        }
                    });
                if let Some(index) = self
                    .selected_review
                    .filter(|index| *index < self.review_lines.len())
                {
                    if self.review_preview.is_none() && self.review_preview_error.is_none() {
                        if let Some(path) =
                            self.processed_path.as_ref().or(self.image_path.as_ref())
                        {
                            match load_review_image(path, self.review_lines[index].bounds) {
                                Ok(image) => {
                                    self.review_preview = Some(ui.ctx().load_texture(
                                        format!("ocr_review_{}", self.task_generation),
                                        image,
                                        TextureOptions::LINEAR,
                                    ))
                                }
                                Err(err) => self.review_preview_error = Some(err),
                            }
                        }
                    }
                    let line = &mut self.review_lines[index];
                    if let Some(texture) = &self.review_preview {
                        let size = texture.size_vec2();
                        let scale = (ui.available_width() / size.x.max(1.0))
                            .min(100.0 / size.y.max(1.0))
                            .min(3.0);
                        ui.add(egui::Image::new((texture.id(), size * scale)));
                    }
                    if let Some(error) = &self.review_preview_error {
                        ui.label(format!("校对图片加载失败：{error}"));
                    }
                    ui.add(
                        TextEdit::singleline(&mut line.text).desired_width(ui.available_width()),
                    );
                    if ui
                        .add_enabled(
                            !line.text.trim().is_empty(),
                            egui::Button::new("确认并加入正文"),
                        )
                        .clicked()
                    {
                        let accepted = line.clone();
                        let accepted_text = accepted.text.trim().to_string();
                        self.undo_state = Some((self.text.clone(), self.text_format));
                        if !self.text.is_empty() && self.text_format != OcrTextFormat::Continuous {
                            self.text.push('\n');
                        }
                        self.text.push_str(&accepted_text);
                        if !self.original_text.is_empty() {
                            self.original_text.push('\n');
                        }
                        self.original_text.push_str(&accepted_text);
                        self.ocr_lines.push(accepted);
                        self.text_format = OcrTextFormat::Custom;
                        self.review_lines.remove(index);
                        self.selected_review = None;
                        self.status = "已加入正文，可继续编辑或手动加入历史。".to_string();
                    }
                }
            });
    }

    fn draw_result_panel(&mut self, ui: &mut egui::Ui) {
        // The central panel owns all remaining horizontal space. Pin this UI
        // to that exact width so framed editors cannot grow beyond the native
        // window when button labels or DPI scaling change their minimum size.
        let panel_width = (ui.available_width() - RESULT_FRAME_EDGE_INSET).max(0.0);
        ui.set_min_width(panel_width);
        ui.set_max_width(panel_width);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("识别结果").strong());
                let history_label = if self.show_history {
                    "隐藏历史"
                } else {
                    "历史"
                };
                // The summary gains backend, model and timing details when OCR
                // finishes. Constrain it so that long metadata cannot make this
                // horizontal row push the native viewport past its right edge.
                let summary = self.result_summary();
                let history_width = if self.show_history { 76.0 } else { 52.0 };
                let summary_width = (ui.available_width() - history_width - 8.0).max(24.0);
                ui.add_sized(
                    [summary_width, ui.spacing().interact_size.y],
                    egui::Label::new(
                        RichText::new(&summary)
                            .small()
                            .color(Color32::from_rgb(100, 116, 139)),
                    )
                    .truncate(),
                )
                .on_hover_text(&summary);
                if ui
                    .add_enabled_ui(!self.busy, |ui| {
                        ui.add_sized(
                            [history_width, ui.spacing().interact_size.y],
                            egui::Button::new(history_label),
                        )
                    })
                    .inner
                    .clicked()
                {
                    self.show_history = !self.show_history;
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for format in OcrTextFormat::ALL {
                    let selected = self.text_format == format;
                    if ui
                        .add_enabled(
                            !self.busy && !self.original_text.is_empty(),
                            egui::SelectableLabel::new(selected, format.label()),
                        )
                        .clicked()
                    {
                        self.apply_text_format(format);
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let has_text = self.has_text();
                let can_edit_text = !self.busy && has_text;
                if ui
                    .add_enabled(
                        !self.busy && self.undo_state.is_some(),
                        egui::Button::new("撤销整理"),
                    )
                    .clicked()
                {
                    if let Some((previous, format)) = self.undo_state.take() {
                        self.text = previous;
                        self.text_format = format;
                        self.focus_result_once = true;
                        self.status = "已撤销上一次整理。".to_string();
                    }
                }
                if ui
                    .add_enabled(
                        !self.busy
                            && !self.original_text.is_empty()
                            && self.text != self.original_text,
                        egui::Button::new("恢复原文"),
                    )
                    .clicked()
                {
                    self.undo_state = Some((self.text.clone(), self.text_format));
                    self.text = self.original_text.clone();
                    self.text_format = OcrTextFormat::Original;
                    self.focus_result_once = true;
                    self.status = "已恢复未经整理的 OCR 原文。".to_string();
                }
                ui.add_enabled_ui(can_edit_text, |ui| {
                    ui.menu_button("更多", |ui| {
                        if ui.button("清理多余空格").clicked() {
                            let transformed = normalize_spaces(&self.text);
                            self.apply_text_transform(transformed, "已清理多余空格。");
                            ui.close_menu();
                        }
                        if ui.button("删除所有横向空格").clicked() {
                            let transformed = remove_spaces(&self.text);
                            self.apply_text_transform(transformed, "已删除所有横向空格，可撤销。");
                            ui.close_menu();
                        }
                        if ui.button("转换为 Markdown 表格").clicked() {
                            let table = table_mode_with_positions(&self.text, &self.ocr_lines);
                            let transformed = table_to_markdown(&table);
                            self.apply_text_transform(transformed, "已转换为 Markdown 表格。");
                            ui.close_menu();
                        }
                    });
                });
                let has_unsaved_changes = self.text.trim() != self.last_saved_text;
                if ui
                    .add_enabled(
                        can_edit_text && has_unsaved_changes,
                        egui::Button::new("保存修改"),
                    )
                    .clicked()
                {
                    self.save_history_now();
                }
            });
            ui.add_space(8.0);
            let review_top = ui.cursor().top();
            self.draw_review_panel(ui);
            let review_expanded = ui.cursor().top() - review_top > 60.0;
            let reserved_for_history = if self.show_history { 156.0 } else { 0.0 };
            let min_editor_height = if review_expanded {
                80.0
            } else if ui.available_height() < 340.0 {
                180.0
            } else {
                260.0
            };
            let editor_height =
                (ui.available_height() - reserved_for_history - 12.0).max(min_editor_height);
            Frame::none()
                .fill(Color32::WHITE)
                .stroke(Stroke::new(1.0, Color32::from_rgb(226, 232, 240)))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(10.0, 10.0))
                .show(ui, |ui| {
                    if self.busy {
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), editor_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.add_space((editor_height * 0.34).min(120.0));
                                ui.spinner();
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new("正在识别...")
                                        .strong()
                                        .color(Color32::from_rgb(51, 65, 85)),
                                );
                            },
                        );
                    } else {
                        let editor_width =
                            (ui.available_width() - RESULT_FRAME_EDGE_INSET).max(24.0);
                        let font_id = egui::TextStyle::Body.resolve(ui.style());
                        let text_color = ui.visuals().text_color();
                        let mut layouter = move |ui: &egui::Ui, text: &str, wrap_width: f32| {
                            let mut job = egui::text::LayoutJob::simple(
                                text.to_owned(),
                                font_id.clone(),
                                text_color,
                                wrap_width.min(editor_width),
                            );
                            job.wrap.break_anywhere = true;
                            ui.fonts(|fonts| fonts.layout_job(job))
                        };
                        let response = ui.add_sized(
                            [editor_width, editor_height],
                            TextEdit::multiline(&mut self.text)
                                .hint_text("识别结果会显示在这里")
                                .desired_width(editor_width)
                                .layouter(&mut layouter),
                        );
                        if response.changed() {
                            self.text_format = OcrTextFormat::Custom;
                        }
                        if self.focus_result_once {
                            response.request_focus();
                            self.focus_result_once = false;
                        }
                    }
                });

            if self.show_history {
                ui.add_space(10.0);
                Frame::none()
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(226, 232, 240)))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(Margin::symmetric(8.0, 8.0))
                    .show(ui, |ui| {
                        ScrollArea::vertical()
                            .id_salt("ocr_history_scroll")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                if self.history.is_empty() {
                                    ui.label(
                                        RichText::new("暂无历史")
                                            .small()
                                            .color(Color32::from_rgb(100, 116, 139)),
                                    );
                                    return;
                                }
                                let mut selected: Option<String> = None;
                                for item in &self.history {
                                    let preview = one_line_preview(&item.text, 58);
                                    if ui
                                        .selectable_label(
                                            false,
                                            format!("{}  {}", item.when, preview),
                                        )
                                        .clicked()
                                    {
                                        selected = Some(item.text.clone());
                                    }
                                }
                                if let Some(text) = selected {
                                    self.original_text = text.clone();
                                    self.text = text.clone();
                                    self.ocr_lines.clear();
                                    self.text_format = OcrTextFormat::Original;
                                    self.undo_state = None;
                                    self.last_saved_text = text.trim().to_string();
                                    self.focus_result_once = true;
                                    self.status = "已载入 OCR 历史。".to_string();
                                }
                            });
                    });
            }
        });
    }
}

impl Drop for OcrApp {
    fn drop(&mut self) {
        self.cleanup_current_images();
    }
}

impl eframe::App for OcrApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_window_request(ctx);
        self.poll_result(ctx);
        ctx.request_repaint_after(if self.busy {
            Duration::from_millis(120)
        } else {
            OCR_REQUEST_POLL_INTERVAL
        });

        let available_width = ctx.available_rect().width();
        let compact_toolbar = available_width < 640.0;
        let toolbar_height = if compact_toolbar { 94.0 } else { 60.0 };
        egui::TopBottomPanel::top("ocr_top")
            .exact_height(toolbar_height)
            .frame(Frame::none().fill(Color32::from_rgb(248, 250, 252)))
            .show(ctx, |ui| {
                ui.add_space(9.0);
                self.draw_toolbar(ui, ctx);
            });

        egui::TopBottomPanel::bottom("ocr_status")
            .exact_height(44.0)
            .frame(Frame::none().fill(Color32::from_rgb(248, 250, 252)))
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(&self.status)
                        .small()
                        .color(Color32::from_rgb(71, 85, 105)),
                );
            });

        // Let egui own the split instead of manually subtracting widths inside
        // a horizontal row. SidePanel reserves one exact rectangle and
        // CentralPanel receives the remainder, which keeps the OCR editor
        // stable across resize, DPI and font-metric changes.
        let available_width = ctx.available_rect().width();
        let left_panel_max = LEFT_PANEL_MAX_WIDTH
            .min((available_width - RESULT_PANEL_TARGET_MIN_WIDTH).max(LEFT_PANEL_MIN_WIDTH));
        let left_panel_default =
            (available_width * 0.38).clamp(LEFT_PANEL_MIN_WIDTH, left_panel_max);
        egui::SidePanel::left("ocr_controls")
            .resizable(true)
            .default_width(left_panel_default)
            .min_width(LEFT_PANEL_MIN_WIDTH)
            .max_width(left_panel_max)
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(248, 250, 252))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(226, 232, 240)))
                    .inner_margin(Margin::symmetric(12.0, 10.0)),
            )
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .id_salt("ocr_controls_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.draw_left_panel(ui));
            });

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(248, 250, 252))
                    .inner_margin(Margin::symmetric(12.0, 10.0)),
            )
            .show(ctx, |ui| self.draw_result_panel(ui));
    }
}

#[cfg(windows)]
fn open_folder(path: &Path) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[cfg(not(windows))]
fn open_folder(path: &Path) -> Result<(), String> {
    let _ = path;
    Err("open folder is only available on Windows".to_string())
}

#[cfg(windows)]
fn capture_screenshot_image(
    target_hwnd: isize,
    mode: CaptureMode,
) -> Result<CaptureResult, String> {
    if mode == CaptureMode::AutoWindow && target_hwnd == 0 {
        return Err("没有可截取的目标窗口。".to_string());
    }

    let output_path = temp_ocr_image_path("capture");
    let _ = fs::remove_file(&output_path);

    let (frame, fallback_reason) = match mode {
        CaptureMode::AutoWindow => match windows_graphics_capture::capture_window(target_hwnd) {
            Ok(frame) => (frame, None),
            Err(wgc_error) => match dxgi_capture::capture_window(target_hwnd) {
                Ok(frame) => (
                    frame,
                    Some(format!("WGC 窗口截图失败，已改用 DXGI：{wgc_error}")),
                ),
                Err(dxgi_error) => {
                    return Err(format!(
                        "窗口截图失败。WGC：{wgc_error}；DXGI：{dxgi_error}"
                    ));
                }
            },
        },
        CaptureMode::ManualRegion => {
            let (desktop, fallback_reason) =
                match windows_graphics_capture::capture_virtual_desktop() {
                    Ok(frame) => (frame, None),
                    Err(wgc_error) => match dxgi_capture::capture_virtual_desktop() {
                        Ok(frame) => (
                            frame,
                            Some(format!("WGC 桌面截图失败，已改用 DXGI：{wgc_error}")),
                        ),
                        Err(dxgi_error) => {
                            return Err(format!(
                                "桌面截图失败。WGC：{wgc_error}；DXGI：{dxgi_error}"
                            ));
                        }
                    },
                };
            let selected = screenshot_region_selector::select_region(&desktop)
                .map_err(|err| format!("打开框选界面失败：{err}"))?
                .ok_or_else(|| "已取消截图。".to_string())?;
            let local_x = i64::from(selected.x) - i64::from(desktop.origin_x);
            let local_y = i64::from(selected.y) - i64::from(desktop.origin_y);
            if local_x < 0
                || local_y < 0
                || local_x + i64::from(selected.width) > i64::from(desktop.width())
                || local_y + i64::from(selected.height) > i64::from(desktop.height())
            {
                return Err("框选区域超出已捕获的虚拟桌面范围。".to_string());
            }
            let cropped = image::imageops::crop_imm(
                &desktop.image,
                local_x as u32,
                local_y as u32,
                selected.width,
                selected.height,
            )
            .to_image();
            let frame = windows_graphics_capture::CapturedFrame {
                image: cropped,
                origin_x: selected.x,
                origin_y: selected.y,
                source: windows_graphics_capture::CapturedSource::DesktopRegion,
                elapsed: desktop.elapsed,
            };
            (frame, fallback_reason)
        }
    };

    windows_graphics_capture::save_png(&frame.image, &output_path)
        .map_err(|err| format!("保存 OCR 截图失败：{err}"))?;
    Ok(CaptureResult {
        path: output_path,
        source: if mode == CaptureMode::AutoWindow {
            CaptureSource::Window
        } else {
            CaptureSource::ManualRegion
        },
        fallback_reason,
    })
}

#[cfg(not(windows))]
fn capture_screenshot_image(
    _target_hwnd: isize,
    _mode: CaptureMode,
) -> Result<CaptureResult, String> {
    Err("OCR 仅支持 Windows。".to_string())
}

#[cfg(windows)]
fn show_ocr_window_best_effort() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, FindWindowW, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST,
        SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW,
    };
    let title = wide(WINDOW_TITLE);
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd != 0 {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(windows))]
fn show_ocr_window_best_effort() {}

fn recognize_from_image(
    original_path: &Path,
    options: OcrProcessOptions,
    mut report_stage: impl FnMut(&'static str),
) -> (
    Option<PathBuf>,
    Result<OcrTextResult, String>,
    Option<DetectedCrop>,
) {
    let started = Instant::now();
    let prepare_started = Instant::now();
    report_stage("正在预处理图片...");
    let prepared = match prepare_image_for_ocr(original_path, options) {
        Ok(prepared) => prepared,
        Err(err) => return (None, Err(err), None),
    };
    runtime_log::log_ocr(
        runtime_log::RuntimeLogLevel::Basic,
        "ocr_preprocess",
        format!(
            "elapsed_ms={} source_pixels={} memory_transport={} options={}",
            prepare_started.elapsed().as_millis(),
            image::image_dimensions(original_path)
                .map(|(width, height)| format!("{width}x{height}"))
                .unwrap_or_else(|_| "unknown".to_string()),
            if prepared.transport_png.is_some() {
                1
            } else {
                0
            },
            options.summary(),
        ),
    );
    let processed_path = if prepared.owned_temp {
        Some(prepared.path.clone())
    } else {
        None
    };
    let detected_crop = prepared.detected_crop;
    report_stage("正在加载模型并检测、识别文字...");
    let mut result = recognize_image_with_rapidocr(
        &prepared.path,
        prepared.transport_png.as_deref(),
        options.language,
        options.profile,
    )
    .map(|mut result| {
        result.elapsed = started.elapsed();
        result.options = options.summary();
        result
    });
    if result
        .as_ref()
        .is_ok_and(|value| value.text.trim().is_empty())
        && detected_crop.is_some_and(|crop| crop.applied)
    {
        report_stage("裁边结果为空，正在使用原图重试...");
        runtime_log::log_ocr(
            runtime_log::RuntimeLogLevel::Basic,
            "ocr_crop_retry",
            "reason=empty_result source=original",
        );
        if let Ok(mut retry) =
            recognize_image_with_rapidocr(original_path, None, options.language, options.profile)
        {
            retry.elapsed = started.elapsed();
            retry.options = format!("{}；裁边为空，已用原图重试", options.summary());
            result = Ok(retry);
        }
    }
    if result.is_err() {
        if let Some(path) = &processed_path {
            let _ = std::fs::remove_file(path);
        }
    }
    (processed_path, result, detected_crop)
}

fn prepare_image_for_ocr(path: &Path, options: OcrProcessOptions) -> Result<PreparedImage, String> {
    if !options.has_image_processing() {
        return Ok(PreparedImage {
            path: path.to_path_buf(),
            owned_temp: false,
            detected_crop: None,
            transport_png: None,
        });
    }

    let mut source_path = path.to_path_buf();
    let mut owned_source = false;
    let mut detected_crop = None;

    if !options.crop.is_full() {
        let image = image::open(path).map_err(|e| format!("读取截图失败：{e}"))?;
        let cropped = crop_image(image, options.crop);
        let crop_path = temp_ocr_image_path("crop");
        cropped
            .save_with_format(&crop_path, image::ImageFormat::Png)
            .map_err(|e| format!("保存裁剪图片失败：{e}"))?;
        source_path = crop_path;
        owned_source = true;
    }

    if options.auto_border_crop {
        if let Ok(Some(crop_result)) = crop_border_with_opencv(&source_path) {
            let apply_crop = crop_result.crop.confidence >= AUTO_CROP_MIN_CONFIDENCE;
            if options.crop.is_full() {
                detected_crop = Some(DetectedCrop {
                    applied: apply_crop,
                    ..crop_result.crop
                });
            }
            if apply_crop {
                if owned_source {
                    let _ = std::fs::remove_file(&source_path);
                }
                source_path = crop_result.path;
                owned_source = true;
            } else {
                let _ = std::fs::remove_file(&crop_result.path);
            }
        }
    }

    if !options.has_post_crop_processing() {
        return Ok(PreparedImage {
            path: source_path,
            owned_temp: owned_source,
            detected_crop,
            transport_png: None,
        });
    }

    let mut image = image::open(&source_path).map_err(|e| format!("读取截图失败：{e}"))?;
    if options.denoise {
        image = image.blur(0.65);
    }
    if options.scale2x {
        let (width, height) = image.dimensions();
        image = image.resize(
            width.saturating_mul(2).max(1),
            height.saturating_mul(2).max(1),
            image::imageops::FilterType::CatmullRom,
        );
    }
    if options.contrast {
        image = image.adjust_contrast(35.0).brighten(8);
    }
    if options.binarize {
        image = binarize_image(image);
    }
    if options.invert {
        image.invert();
    }

    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("编码预处理图片失败：{e}"))?;
    let processed = temp_ocr_image_path("processed");
    fs::write(&processed, &png).map_err(|e| format!("保存预处理图片失败：{e}"))?;
    if owned_source {
        let _ = std::fs::remove_file(&source_path);
    }
    Ok(PreparedImage {
        path: processed,
        owned_temp: true,
        detected_crop,
        transport_png: Some(BASE64.encode(png)),
    })
}

fn crop_image(image: DynamicImage, mut crop: CropRect) -> DynamicImage {
    crop.normalize();
    if crop.is_full() {
        return image;
    }
    let (width, height) = image.dimensions();
    let x = ((width as f32) * crop.left).round() as u32;
    let y = ((height as f32) * crop.top).round() as u32;
    let right = ((width as f32) * crop.right).round() as u32;
    let bottom = ((height as f32) * crop.bottom).round() as u32;
    let crop_width = right.saturating_sub(x).max(1).min(width.saturating_sub(x));
    let crop_height = bottom
        .saturating_sub(y)
        .max(1)
        .min(height.saturating_sub(y));
    image.crop_imm(x, y, crop_width, crop_height)
}

fn binarize_image(image: DynamicImage) -> DynamicImage {
    let gray = image.to_luma8();
    let mut out = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(gray.width(), gray.height());
    for (x, y, pixel) in gray.enumerate_pixels() {
        let value = if pixel[0] >= 160 { 255 } else { 0 };
        out.put_pixel(x, y, Rgba([value, value, value, 255]));
    }
    DynamicImage::ImageRgba8(out)
}

fn temp_ocr_image_path(label: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d%H%M%S%3f");
    std::env::temp_dir().join(format!("kaixin_ocr_{label}_{stamp}.png"))
}

struct OcrScreenshotPrefs {
    auto_save: bool,
    save_dir: Option<PathBuf>,
    name_pattern: String,
}

fn save_ocr_capture_if_enabled(path: &Path) -> OcrCaptureSaveNote {
    let prefs = ocr_screenshot_prefs();
    if !prefs.auto_save {
        return OcrCaptureSaveNote::default();
    }
    match next_ocr_screenshot_path(&prefs).and_then(|dest| {
        fs::copy(path, &dest)
            .map(|_| dest)
            .map_err(|e| format!("保存 OCR 截图失败：{e}"))
    }) {
        Ok(saved_path) => {
            let _ = screenshot_store::record_screenshot(&screenshot_store::ScreenshotRecord {
                path: saved_path.clone(),
                source: "ocr_capture".to_string(),
                ..screenshot_store::ScreenshotRecord::default()
            });
            OcrCaptureSaveNote {
                saved_path: Some(saved_path),
                error: None,
            }
        }
        Err(error) => OcrCaptureSaveNote {
            saved_path: None,
            error: Some(error),
        },
    }
}

fn ocr_capture_status(session: Option<&CaptureSession>) -> String {
    let mut status = if let Some(session) = session {
        let mut value = if let Some(file_name) = session.input_file_name() {
            format!("已接收截图“{file_name}”，预览已加载，正在自动识别文字...")
        } else {
            format!("{}已完成，正在识别文字...", session.source_label())
        };
        if session.target_hwnd != 0 {
            value.push_str(" 目标窗口已绑定。");
        }
        if let Some(reason) = session
            .fallback_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let label = if session.source == CaptureSource::FallbackRegion {
                "回退原因"
            } else {
                "说明"
            };
            value.push_str(&format!(" {label}：{reason}。"));
        }
        value
    } else {
        "已完成截图，正在识别文字...".to_string()
    };
    if let Some(note) = session.map(|session| &session.save_note) {
        if note.saved_path.is_some() {
            status.push_str(" 原图已自动保存。");
        } else if let Some(error) = &note.error {
            status.push_str(&format!(" 原图自动保存失败：{error}"));
        }
    }
    status
}

fn window_capture_failed_status(err: &str) -> String {
    format!(
        "窗口截图失败：{err}。可点“手动框选”改用配置的框选方案，或切回目标窗口后点“当前窗口截图”。"
    )
}

fn ocr_screenshot_prefs() -> OcrScreenshotPrefs {
    let content = app_paths::read_config_text(config_path()).ok();
    let auto_save = content
        .as_deref()
        .and_then(|text| ini_value(text, "ocr", "screenshot_auto_save"))
        .as_deref()
        .map(|value| parse_bool_config(value, true))
        .unwrap_or(true);
    let save_dir = content
        .as_deref()
        .and_then(|text| ini_value(text, "ocr", "screenshot_save_dir"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let name_pattern = content
        .as_deref()
        .and_then(|text| ini_value(text, "ocr", "screenshot_name_pattern"))
        .unwrap_or_else(|| "ocr-{datetime}".to_string());
    OcrScreenshotPrefs {
        auto_save,
        save_dir,
        name_pattern,
    }
}

fn next_ocr_screenshot_path(prefs: &OcrScreenshotPrefs) -> Result<PathBuf, String> {
    let dir = prefs
        .save_dir
        .clone()
        .unwrap_or_else(default_ocr_screenshot_dir);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建 OCR 截图目录失败：{}: {e}", dir.display()))?;
    let base_stem = render_screenshot_name_pattern(&prefs.name_pattern, 1, "ocr-screenshot");
    let mut path = dir.join(format!("{base_stem}.png"));
    for idx in 2..1000 {
        if !path.exists() {
            return Ok(path);
        }
        let stem = if prefs.name_pattern.contains("{seq}") {
            render_screenshot_name_pattern(&prefs.name_pattern, idx, "ocr-screenshot")
        } else {
            format!("{base_stem}_{idx:03}")
        };
        path = dir.join(format!("{stem}.png"));
    }
    Ok(path)
}

fn default_ocr_screenshot_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|p| p.join("Pictures").join("Kaixin OCR"))
        .or_else(|| app_paths::local_data_dir().map(|p| p.join("ocr-screenshots")))
        .unwrap_or_else(|| std::env::temp_dir().join("kaixin-ocr"))
}

fn render_screenshot_name_pattern(pattern: &str, seq: usize, fallback: &str) -> String {
    let now = chrono::Local::now();
    let pattern = if pattern.trim().is_empty() {
        "ocr-{datetime}"
    } else {
        pattern.trim()
    };
    let rendered = pattern
        .replace("{datetime}", &now.format("%Y%m%d_%H%M%S").to_string())
        .replace("{date}", &now.format("%Y%m%d").to_string())
        .replace("{time}", &now.format("%H%M%S").to_string())
        .replace("{year}", &now.format("%Y").to_string())
        .replace("{month}", &now.format("%m").to_string())
        .replace("{day}", &now.format("%d").to_string())
        .replace("{hour}", &now.format("%H").to_string())
        .replace("{minute}", &now.format("%M").to_string())
        .replace("{second}", &now.format("%S").to_string())
        .replace("{ms}", &now.format("%3f").to_string())
        .replace("{seq}", &format!("{seq:03}"));
    let sanitized = sanitize_filename_stem(&rendered);
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

fn sanitize_filename_stem(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim_matches([' ', '.']).to_string();
    if trimmed.eq_ignore_ascii_case("png")
        || trimmed.eq_ignore_ascii_case("jpg")
        || trimmed.eq_ignore_ascii_case("jpeg")
    {
        return String::new();
    }
    for ext in [".png", ".jpg", ".jpeg"] {
        if trimmed.to_ascii_lowercase().ends_with(ext) {
            let end = trimmed.len().saturating_sub(ext.len());
            return trimmed[..end].trim_matches([' ', '.']).to_string();
        }
    }
    trimmed
}

fn ocr_speed_profile() -> OcrSpeedProfile {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|text| ini_value(&text, "ocr", "profile"))
        .map(|value| OcrSpeedProfile::from_config(&value))
        .unwrap_or(OcrSpeedProfile::Balanced)
}

fn config_path() -> PathBuf {
    app_paths::config_ini_path().unwrap_or_else(|| {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(app_paths::CONFIG_FILE_NAME)
    })
}

fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case(key) {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn parse_bool_config(value: &str, fallback: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" | "开启" | "开" => true,
        "0" | "false" | "no" | "off" | "disabled" | "关闭" | "关" => false,
        _ => fallback,
    }
}

// Upload only the selected text crop: long captures can exceed GPU texture limits.
fn load_review_image(path: &Path, bounds: Option<[[f32; 2]; 4]>) -> Result<ColorImage, String> {
    let mut image = image::open(path).map_err(|e| format!("读取校对图片失败：{e}"))?;
    if let Some(points) = bounds {
        let width = image.width() as f32;
        let height = image.height() as f32;
        let left = (points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min) - 8.0)
            .clamp(0.0, width) as u32;
        let top = (points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min) - 8.0)
            .clamp(0.0, height) as u32;
        let right = (points
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            + 8.0)
            .clamp(left as f32, width)
            .ceil() as u32;
        let bottom = (points
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            + 8.0)
            .clamp(top as f32, height)
            .ceil() as u32;
        if right <= left || bottom <= top {
            return Err("文字区域超出图片范围".to_string());
        }
        image = image.crop_imm(left, top, right - left, bottom - top);
    }
    let image = image.thumbnail(2048, 256).to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

fn load_color_image(path: &Path) -> Result<(ColorImage, [usize; 2]), String> {
    let image = image::open(path)
        .map_err(|e| format!("读取图片失败：{e}"))?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    Ok((ColorImage::from_rgba_unmultiplied(size, &pixels), size))
}

#[derive(Deserialize)]
struct CvCropPayload {
    ok: bool,
    cropped: Option<bool>,
    output: Option<String>,
    rect: Option<[f32; 4]>,
    width: Option<f32>,
    height: Option<f32>,
    confidence: Option<f32>,
    error: Option<String>,
    traceback: Option<String>,
}

struct CvCropResult {
    path: PathBuf,
    crop: DetectedCrop,
}

fn crop_border_with_opencv(path: &Path) -> Result<Option<CvCropResult>, String> {
    let output = temp_ocr_image_path("border");
    let command_output =
        run_rapidocr_crop_with_timeout(path, &output, OPENCV_CROP_PROCESS_TIMEOUT)?;
    let stdout = String::from_utf8_lossy(&command_output.stdout);
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Err(format!(
            "OpenCV 边框裁剪没有返回 JSON，进程状态：{}",
            command_output.status_label
        ));
    }

    let payload: CvCropPayload = serde_json::from_str(stdout)
        .map_err(|err| format!("解析 OpenCV 边框裁剪输出失败：{err}"))?;

    if !payload.ok || !command_output.success {
        let mut message = payload
            .error
            .clone()
            .unwrap_or_else(|| format!("OpenCV 边框裁剪返回 {}", command_output.status_label));
        if let Some(traceback) = payload
            .traceback
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            message.push('\n');
            message.push_str(traceback.trim());
        }
        let _ = std::fs::remove_file(&output);
        return Err(message);
    }

    let output_path = if payload.cropped.unwrap_or(false) && output.is_file() {
        Some(output.clone())
    } else {
        payload
            .output
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_file())
    };
    if let Some(output_path) = output_path {
        if let Some(crop) = detected_crop_from_payload(&payload) {
            return Ok(Some(CvCropResult {
                path: output_path,
                crop,
            }));
        }
    }
    let _ = std::fs::remove_file(&output);
    Ok(None)
}

fn detected_crop_from_payload(payload: &CvCropPayload) -> Option<DetectedCrop> {
    let [x, y, width, height] = payload.rect?;
    let image_width = payload.width?.max(1.0);
    let image_height = payload.height?.max(1.0);
    if width <= 1.0 || height <= 1.0 {
        return None;
    }
    Some(DetectedCrop {
        rect: CropRect {
            left: (x / image_width).clamp(0.0, 1.0),
            top: (y / image_height).clamp(0.0, 1.0),
            right: ((x + width) / image_width).clamp(0.0, 1.0),
            bottom: ((y + height) / image_height).clamp(0.0, 1.0),
        },
        confidence: payload.confidence.unwrap_or(0.0).max(0.0),
        applied: false,
    })
}

fn run_rapidocr_crop_with_timeout(
    input_path: &Path,
    output_path: &Path,
    timeout: Duration,
) -> Result<RapidOcrCommandOutput, String> {
    let image = input_path.to_string_lossy().to_string();
    let output = output_path.to_string_lossy().to_string();
    let request = RapidOcrCropRequest {
        command: "crop",
        image: &image,
        output: &output,
    };
    let slot = rapidocr_process_slot();
    let mut guard = slot
        .lock()
        .map_err(|err| format!("RapidOCR 常驻裁剪进程锁定失败：{err}"))?;
    if guard.is_none() {
        *guard = Some(spawn_rapidocr_process()?);
    }
    let result = guard
        .as_mut()
        .expect("rapidocr process initialized for crop")
        .request(&request, timeout);
    if result.is_err() {
        if let Some(mut process) = guard.take() {
            process.terminate();
        }
    }
    result
}

#[derive(Clone, Deserialize)]
struct OcrReviewLine {
    text: String,
    score: f32,
    #[serde(rename = "box")]
    bounds: Option<[[f32; 2]; 4]>,
}

#[derive(Deserialize)]
struct RapidOcrPayload {
    ok: bool,
    engine: Option<String>,
    backend: Option<String>,
    profile: Option<String>,
    ocr_version: Option<String>,
    model_type: Option<String>,
    language: Option<String>,
    init_ms: Option<f64>,
    load_ms: Option<f64>,
    infer_ms: Option<f64>,
    total_ms: Option<f64>,
    preprocess_ms: Option<f64>,
    #[serde(default)]
    attempts: Vec<serde_json::Value>,
    tile_count: Option<usize>,
    #[serde(default)]
    lines: Vec<OcrReviewLine>,
    #[serde(default)]
    low_confidence_lines: Vec<OcrReviewLine>,
    text: Option<String>,
    error: Option<String>,
    traceback: Option<String>,
}

struct RapidOcrCommandOutput {
    status_label: String,
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl RapidOcrCommandOutput {
    fn from_output(output: Output) -> Self {
        Self {
            status_label: output.status.to_string(),
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

#[derive(Serialize)]
struct RapidOcrStdioRequest<'a> {
    image: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_bytes_base64: Option<&'a str>,
    lang: &'static str,
    min_score: f32,
    profile: &'static str,
    max_side_len: u32,
}

#[derive(Serialize)]
struct RapidOcrWarmupRequest {
    command: &'static str,
    lang: &'static str,
    profile: &'static str,
}

#[derive(Serialize)]
struct RapidOcrCropRequest<'a> {
    command: &'static str,
    image: &'a str,
    output: &'a str,
}

struct RapidOcrProcess {
    last_used: Instant,
    child: Child,
    stdin: ChildStdin,
    stdout: Option<BufReader<ChildStdout>>,
    stderr_tail: Arc<Mutex<VecDeque<u8>>>,
    #[cfg(windows)]
    _job: Option<ChildProcessJob>,
}

fn rapidocr_process_slot() -> &'static Mutex<Option<RapidOcrProcess>> {
    static PROCESS: OnceLock<Mutex<Option<RapidOcrProcess>>> = OnceLock::new();
    PROCESS.get_or_init(|| {
        // The reaper never waits for a busy inference or runs on the UI thread.
        let _ = std::thread::Builder::new()
            .name("kaixin-ocr-idle".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(30));
                let Some(slot) = PROCESS.get() else { continue };
                let Ok(mut guard) = slot.try_lock() else {
                    continue;
                };
                if guard
                    .as_ref()
                    .is_some_and(|process| process.last_used.elapsed() >= Duration::from_secs(180))
                {
                    let process = guard.take();
                    drop(guard);
                    if let Some(mut process) = process {
                        process.terminate();
                    }
                    runtime_log::log_ocr(
                        runtime_log::RuntimeLogLevel::Basic,
                        "rapidocr_idle_release",
                        "idle_secs=180",
                    );
                }
            });
        Mutex::new(None)
    })
}

impl RapidOcrProcess {
    fn request<T: Serialize>(
        &mut self,
        request: &T,
        timeout: Duration,
    ) -> Result<RapidOcrCommandOutput, String> {
        self.last_used = Instant::now();
        serde_json::to_writer(&mut self.stdin, request)
            .map_err(|err| format!("写入 RapidOCR 常驻请求失败：{err}"))?;
        self.stdin
            .write_all(b"\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|err| format!("发送 RapidOCR 常驻请求失败：{err}"))?;

        let mut stdout = self
            .stdout
            .take()
            .ok_or_else(|| "RapidOCR 常驻进程 stdout 不可用。".to_string())?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = stdout.read_line(&mut line).map(|read| (read, line));
            let _ = tx.send((stdout, result));
        });

        let (stdout, result) = match rx.recv_timeout(timeout) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.terminate();
                return Err(format!(
                    "RapidOCR 常驻进程等待响应超时（{} 秒），已重启。stderr: {}",
                    timeout.as_secs(),
                    self.stderr_snapshot()
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(format!(
                    "RapidOCR 常驻进程读取线程退出。stderr: {}",
                    self.stderr_snapshot()
                ));
            }
        };
        self.stdout = Some(stdout);
        self.last_used = Instant::now();

        let (read, line) = result.map_err(|err| {
            format!(
                "读取 RapidOCR 常驻响应失败：{err}；stderr: {}",
                self.stderr_snapshot()
            )
        })?;
        if read == 0 {
            return Err(format!(
                "RapidOCR 常驻进程已退出，没有返回结果。stderr: {}",
                self.stderr_snapshot()
            ));
        }
        Ok(RapidOcrCommandOutput {
            status_label: "stdio".to_string(),
            success: true,
            stdout: line.into_bytes(),
            stderr: self.stderr_snapshot().into_bytes(),
        })
    }

    fn stderr_snapshot(&self) -> String {
        let Ok(tail) = self.stderr_tail.lock() else {
            return "<stderr buffer unavailable>".to_string();
        };
        let bytes = tail.iter().copied().collect::<Vec<_>>();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }

    fn terminate(&mut self) {
        let pid = self.child.id();
        #[cfg(windows)]
        if let Some(job) = &self._job {
            job.terminate();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = ACTIVE_RAPIDOCR_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::Relaxed);
    }
}

#[cfg(windows)]
fn terminate_active_rapidocr_process() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    let pid = ACTIVE_RAPIDOCR_PID.swap(0, Ordering::SeqCst);
    if pid == 0 {
        return;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle != 0 {
            let _ = TerminateProcess(handle, 1223);
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
fn terminate_active_rapidocr_process() {}

fn recognize_image_with_rapidocr(
    path: &Path,
    image_bytes_base64: Option<&str>,
    language: OcrLanguageMode,
    profile: OcrSpeedProfile,
) -> Result<OcrTextResult, String> {
    let started = Instant::now();
    runtime_log::log_ocr(
        runtime_log::RuntimeLogLevel::Basic,
        "rapidocr_start",
        format!(
            "image={} memory_image={} language={} profile={} max_side_len={} timeout_secs={}",
            runtime_log::path_for_log(path),
            if image_bytes_base64.is_some() { 1 } else { 0 },
            language.rapidocr_lang(),
            profile.as_config(),
            profile.max_side_len(),
            ocr_request_timeout(path, profile, false).as_secs()
        ),
    );
    let output = match run_rapidocr_command_with_timeout(
        path,
        image_bytes_base64,
        language,
        profile,
        ocr_request_timeout(path, profile, false),
    ) {
        Ok(output) => {
            runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Basic,
                "rapidocr_exit",
                format!(
                    "status={} elapsed_ms={} stdout_bytes={} stderr_bytes={}",
                    output.status_label,
                    started.elapsed().as_millis(),
                    output.stdout.len(),
                    output.stderr.len()
                ),
            );
            output
        }
        Err(err) => {
            runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Error,
                "rapidocr_exit",
                format!(
                    "status=failed elapsed_ms={} reason={}",
                    started.elapsed().as_millis(),
                    log_error_detail(&err)
                ),
            );
            return Err(err);
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = stdout.trim();
    if stdout.is_empty() {
        let message = if stderr.is_empty() {
            format!("RapidOCR 没有返回 JSON，进程状态：{}", output.status_label)
        } else {
            format!("RapidOCR 没有返回 JSON：{stderr}")
        };
        runtime_log::log_ocr(
            runtime_log::RuntimeLogLevel::Error,
            "rapidocr_result",
            format!(
                "status=failed elapsed_ms={} reason={}",
                started.elapsed().as_millis(),
                log_error_detail(&message)
            ),
        );
        return Err(message);
    }

    let payload: RapidOcrPayload = match serde_json::from_str(stdout) {
        Ok(payload) => payload,
        Err(err) => {
            let message = if stderr.is_empty() {
                format!("解析 RapidOCR 输出失败：{err}")
            } else {
                format!("解析 RapidOCR 输出失败：{err}；stderr：{stderr}")
            };
            runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Error,
                "rapidocr_result",
                format!(
                    "status=parse_failed elapsed_ms={} reason={}",
                    started.elapsed().as_millis(),
                    log_error_detail(&message)
                ),
            );
            return Err(message);
        }
    };

    if !payload.ok || !output.success {
        let mut message = payload
            .error
            .clone()
            .unwrap_or_else(|| format!("RapidOCR 返回 {}", output.status_label));
        if message.trim().is_empty() {
            message = format!("RapidOCR 返回 {}", output.status_label);
        }
        if let Some(traceback) = payload
            .traceback
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            message.push('\n');
            message.push_str(traceback.trim());
        }
        if !stderr.is_empty() {
            message.push_str("\nstderr: ");
            message.push_str(&stderr);
        }
        runtime_log::log_ocr(
            runtime_log::RuntimeLogLevel::Error,
            "rapidocr_result",
            format!(
                "status=failed elapsed_ms={} reason={}",
                started.elapsed().as_millis(),
                log_error_detail(&message)
            ),
        );
        return Err(message);
    }

    let engine = rapidocr_engine_label(&payload);
    runtime_log::log_ocr(
        runtime_log::RuntimeLogLevel::Basic,
        "rapidocr_attempts",
        format!(
            "tiles={} attempts={}",
            payload.tile_count.unwrap_or(1),
            serde_json::to_string(&payload.attempts).unwrap_or_default()
        ),
    );
    let text = payload.text.unwrap_or_default().replace("\r\n", "\n");
    runtime_log::log_ocr(
        runtime_log::RuntimeLogLevel::Basic,
        "rapidocr_result",
        format!(
            "status=ok elapsed_ms={} init_ms={} load_ms={} infer_ms={} preprocess_ms={} worker_total_ms={} transport_overhead_ms={} chars={} lines={} engine={}",
            started.elapsed().as_millis(),
            payload.init_ms.unwrap_or_default(),
            payload.load_ms.unwrap_or_default(),
            payload.infer_ms.unwrap_or_default(),
            payload.preprocess_ms.unwrap_or_default(),
            payload.total_ms.unwrap_or_default(),
            (started.elapsed().as_secs_f64() * 1000.0
                - payload.total_ms.unwrap_or_else(|| payload.init_ms.unwrap_or_default()
                    + payload.load_ms.unwrap_or_default() + payload.infer_ms.unwrap_or_default()))
                .max(0.0),
            text.chars().count(),
            text.lines().filter(|line| !line.trim().is_empty()).count(),
            engine
        ),
    );
    Ok(OcrTextResult {
        text,
        lines: payload.lines,
        low_confidence_lines: payload.low_confidence_lines,
        engine,
        elapsed: Duration::ZERO,
        options: String::new(),
    })
}

fn log_error_detail(value: &str) -> String {
    let compact = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    let mut out = compact.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > 400 {
        out = out.chars().take(400).collect::<String>();
        out.push_str("...");
    }
    out
}

fn ocr_request_timeout(path: &Path, profile: OcrSpeedProfile, cold_start: bool) -> Duration {
    let mut seconds = RAPIDOCR_HOT_TIMEOUT.as_secs();
    if matches!(profile, OcrSpeedProfile::Accurate) {
        seconds += 15;
    }
    if let Ok((width, height)) = image::image_dimensions(path) {
        let megapixels = u64::from(width) * u64::from(height) / 1_000_000;
        seconds += megapixels.min(20);
        let limit = profile.max_side_len();
        if width.max(height) > limit
            && u64::from(width.max(height)) >= u64::from(width.min(height)) * 2
        {
            let stride = limit - 192.min(limit / 4);
            let columns = 1 + width.saturating_sub(limit).div_ceil(stride);
            let rows = 1 + height.saturating_sub(limit).div_ceil(stride);
            seconds += (u64::from(columns) * u64::from(rows)).saturating_sub(1) * 8;
        }
    }
    if cold_start {
        seconds = seconds.max(RAPIDOCR_COLD_TIMEOUT.as_secs());
    }
    Duration::from_secs(seconds)
}

fn run_rapidocr_command_with_timeout(
    image_path: &Path,
    image_bytes_base64: Option<&str>,
    language: OcrLanguageMode,
    profile: OcrSpeedProfile,
    timeout: Duration,
) -> Result<RapidOcrCommandOutput, String> {
    if rapidocr_persistent_disabled() {
        shutdown_rapidocr_process();
        return run_rapidocr_oneshot_command_with_timeout(image_path, language, profile, timeout)
            .map(RapidOcrCommandOutput::from_output);
    }

    match run_rapidocr_persistent_command_with_timeout(
        image_path,
        image_bytes_base64,
        language,
        profile,
        timeout,
    ) {
        Ok(output) => Ok(output),
        Err(err) => {
            if err.contains("超时") || err.contains("已取消") {
                return Err(err);
            }
            runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Basic,
                "rapidocr_worker",
                format!(
                    "status=fallback mode=oneshot reason={}",
                    log_error_detail(&err)
                ),
            );
            run_rapidocr_oneshot_command_with_timeout(image_path, language, profile, timeout)
                .map(RapidOcrCommandOutput::from_output)
        }
    }
}

fn shutdown_rapidocr_process() {
    let Ok(mut guard) = rapidocr_process_slot().lock() else {
        return;
    };
    if let Some(mut process) = guard.take() {
        process.terminate();
    }
}

fn start_rapidocr_prewarm(language: OcrLanguageMode, profile: OcrSpeedProfile) {
    if rapidocr_persistent_disabled() {
        return;
    }
    std::thread::spawn(move || {
        let started = Instant::now();
        let result = run_rapidocr_warmup(language, profile);
        match result {
            Ok(output) => runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Basic,
                "rapidocr_warmup",
                format!(
                    "status={} elapsed_ms={} profile={} language={}",
                    output.status_label,
                    started.elapsed().as_millis(),
                    profile.as_config(),
                    language.rapidocr_lang()
                ),
            ),
            Err(err) => runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Error,
                "rapidocr_warmup",
                format!(
                    "status=failed elapsed_ms={} profile={} reason={}",
                    started.elapsed().as_millis(),
                    profile.as_config(),
                    log_error_detail(&err)
                ),
            ),
        }
    });
}

fn run_rapidocr_warmup(
    language: OcrLanguageMode,
    profile: OcrSpeedProfile,
) -> Result<RapidOcrCommandOutput, String> {
    let request = RapidOcrWarmupRequest {
        command: "warmup",
        lang: language.rapidocr_lang(),
        profile: profile.as_config(),
    };
    let slot = rapidocr_process_slot();
    let mut guard = slot
        .lock()
        .map_err(|err| format!("RapidOCR 预热进程锁定失败：{err}"))?;
    if guard.is_none() {
        *guard = Some(spawn_rapidocr_process()?);
    }
    let result = guard
        .as_mut()
        .expect("rapidocr process initialized for warmup")
        .request(&request, RAPIDOCR_COLD_TIMEOUT)
        .and_then(|output| {
            let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
                .map_err(|err| format!("解析 RapidOCR 预热响应失败：{err}"))?;
            if payload.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                return Ok(output);
            }
            Err(payload
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(|message| format!("RapidOCR 预热失败：{message}"))
                .unwrap_or_else(|| "RapidOCR 预热返回失败状态。".to_string()))
        });
    if result.is_err() {
        if let Some(mut process) = guard.take() {
            process.terminate();
        }
    }
    result
}

fn run_rapidocr_persistent_command_with_timeout(
    image_path: &Path,
    image_bytes_base64: Option<&str>,
    language: OcrLanguageMode,
    profile: OcrSpeedProfile,
    timeout: Duration,
) -> Result<RapidOcrCommandOutput, String> {
    let image = image_path.to_string_lossy().to_string();
    let request = RapidOcrStdioRequest {
        image: &image,
        image_bytes_base64,
        lang: language.rapidocr_lang(),
        min_score: 0.5,
        profile: profile.as_config(),
        max_side_len: profile.max_side_len(),
    };
    let slot = rapidocr_process_slot();
    let mut guard = slot
        .lock()
        .map_err(|err| format!("RapidOCR 常驻进程锁定失败：{err}"))?;

    let cold_start = guard.is_none();
    if cold_start {
        *guard = Some(spawn_rapidocr_process()?);
    }

    let timeout = ocr_request_timeout(image_path, profile, cold_start).max(timeout);

    let first = guard
        .as_mut()
        .expect("rapidocr process initialized")
        .request(&request, timeout);
    match first {
        Ok(output) => Ok(output),
        Err(first_err) => {
            let was_cancelled = ACTIVE_RAPIDOCR_PID.load(Ordering::SeqCst) == 0;
            if let Some(mut process) = guard.take() {
                process.terminate();
            }
            if first_err.contains("超时") {
                return Err(first_err);
            }
            if was_cancelled {
                return Err("OCR 任务已取消。".to_string());
            }
            *guard = Some(spawn_rapidocr_process()?);
            guard
                .as_mut()
                .expect("rapidocr process restarted")
                .request(&request, timeout)
                .map_err(|second_err| {
                    format!("RapidOCR 常驻进程失败：{first_err}；重启后仍失败：{second_err}")
                })
        }
    }
}

fn spawn_rapidocr_process() -> Result<RapidOcrProcess, String> {
    let rapidocr_root =
        rapidocr_paths::rapidocr_root().ok_or_else(rapidocr_paths::missing_root_message)?;
    rapidocr_paths::validate_rapidocr_models_cached(&rapidocr_root)?;
    let helper = rapidocr_paths::ocr_helper_path()
        .ok_or_else(|| "RapidOCR helper is unavailable: expected .\\tools\\kaixin_ocr_engine.py inside the installed program directory.".to_string())?;
    let python = rapidocr_paths::python_runtime(Some(&rapidocr_root))?;
    let mut command = Command::new(&python.executable);
    rapidocr_paths::apply_python_runtime_env(&mut command, &python);
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .arg(helper)
        .arg("--stdio")
        .arg("--rapidocr-root")
        .arg(&rapidocr_root)
        .arg("--provider")
        .arg(tool_prefs::ocr_execution_provider())
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::piped());

    #[cfg(windows)]
    let job = ChildProcessJob::new();
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 RapidOCR 常驻进程失败：{e}"))?;
    ACTIVE_RAPIDOCR_PID.store(child.id(), Ordering::SeqCst);
    #[cfg(windows)]
    if let Some(job) = &job {
        job.assign(&child);
    }
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "RapidOCR 常驻进程 stdin 不可用。".to_string())?;
    let stdout = child
        .stdout
        .take()
        .map(BufReader::new)
        .ok_or_else(|| "RapidOCR 常驻进程 stdout 不可用。".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "RapidOCR 常驻进程 stderr 不可用。".to_string())?;
    let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(RAPIDOCR_STDERR_LIMIT)));
    let stderr_tail_writer = Arc::clone(&stderr_tail);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut chunk = [0_u8; 1024];
        while let Ok(read) = reader.read(&mut chunk) {
            if read == 0 {
                break;
            }
            let Ok(mut tail) = stderr_tail_writer.lock() else {
                break;
            };
            for byte in &chunk[..read] {
                if tail.len() == RAPIDOCR_STDERR_LIMIT {
                    tail.pop_front();
                }
                tail.push_back(*byte);
            }
        }
    });
    Ok(RapidOcrProcess {
        last_used: Instant::now(),
        child,
        stdin,
        stdout: Some(stdout),
        stderr_tail,
        #[cfg(windows)]
        _job: job,
    })
}

fn run_rapidocr_oneshot_command_with_timeout(
    image_path: &Path,
    language: OcrLanguageMode,
    profile: OcrSpeedProfile,
    timeout: Duration,
) -> Result<Output, String> {
    let rapidocr_root =
        rapidocr_paths::rapidocr_root().ok_or_else(rapidocr_paths::missing_root_message)?;
    rapidocr_paths::validate_rapidocr_models_cached(&rapidocr_root)?;
    let helper = rapidocr_paths::ocr_helper_path()
        .ok_or_else(|| "RapidOCR helper is unavailable: expected .\\tools\\kaixin_ocr_engine.py inside the installed program directory.".to_string())?;
    let python = rapidocr_paths::python_runtime(Some(&rapidocr_root))?;
    let mut command = Command::new(&python.executable);
    rapidocr_paths::apply_python_runtime_env(&mut command, &python);
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .arg(helper)
        .arg(image_path)
        .arg("--rapidocr-root")
        .arg(&rapidocr_root)
        .arg("--lang")
        .arg(language.rapidocr_lang())
        .arg("--profile")
        .arg(profile.as_config())
        .arg("--max-side-len")
        .arg(profile.max_side_len().to_string())
        .arg("--provider")
        .arg(tool_prefs::ocr_execution_provider())
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    run_command_with_timeout(command, timeout, "RapidOCR")
}

fn rapidocr_persistent_disabled() -> bool {
    if !tool_prefs::ocr_keep_alive_enabled() {
        return true;
    }
    std::env::var("KAIXIN_OCR_ENGINE")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "off" | "false" | "disabled" | "disable" | "oneshot" | "one-shot"
            )
        })
        .unwrap_or(false)
}

fn rapidocr_engine_label(payload: &RapidOcrPayload) -> String {
    let mut parts = vec![payload.engine.as_deref().unwrap_or("RapidOCR").to_string()];
    for value in [
        payload.backend.as_deref(),
        payload.ocr_version.as_deref(),
        payload.profile.as_deref(),
        payload.model_type.as_deref(),
        payload.language.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !value.trim().is_empty() {
            parts.push(value.to_string());
        }
    }
    if let Some(ms) = payload.init_ms {
        parts.push(format!("初始化 {:.0} ms", ms));
    }
    if let Some(ms) = payload.load_ms {
        parts.push(format!("读图 {:.0} ms", ms));
    }
    if let Some(ms) = payload.infer_ms {
        parts.push(format!("识别 {:.0} ms", ms));
    }
    parts.join(" / ")
}

#[cfg(windows)]
struct ChildProcessJob {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ChildProcessJob {
    fn new() -> Option<Self> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle == 0 {
            return None;
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            unsafe {
                CloseHandle(handle);
            }
            return None;
        }

        Some(Self { handle })
    }

    fn assign(&self, child: &std::process::Child) {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        let process = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        if process != 0 {
            unsafe {
                let _ = AssignProcessToJobObject(self.handle, process);
            }
        }
    }

    fn terminate(&self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        unsafe {
            let _ = TerminateJobObject(self.handle, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for ChildProcessJob {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        if self.handle != 0 {
            unsafe {
                CloseHandle(self.handle);
            }
            self.handle = 0;
        }
    }
}

fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
    process_label: &str,
) -> Result<Output, String> {
    #[cfg(windows)]
    let job = ChildProcessJob::new();

    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 {process_label} 失败：{e}"))?;
    #[cfg(windows)]
    if let Some(job) = &job {
        job.assign(&child);
    }

    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return child.wait_with_output().map_err(|e| e.to_string());
        }
        if Instant::now() >= deadline {
            #[cfg(windows)]
            if let Some(job) = &job {
                job.terminate();
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{process_label} 超时（{} 秒），请重试。",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}

#[cfg(windows)]
fn copy_to_system_clipboard(text: &str) -> Result<(), String> {
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text)
        .map_err(|e| format!("copy clipboard text: {e}"))
}

#[cfg(not(windows))]
fn copy_to_system_clipboard(_text: &str) -> Result<(), String> {
    Err("clipboard is Windows-only".to_string())
}

#[path = "srf_ime_ocr/history_store.rs"]
mod history_store;
use history_store::*;

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn ocr_window_request_dir() -> PathBuf {
    app_paths::local_data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join(app_paths::APP_PATH_NAME))
        .join("ocr-window-requests")
}

fn write_ocr_window_request(request: &OcrWindowRequest) -> Result<PathBuf, String> {
    let dir = ocr_window_request_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建 OCR 请求目录失败：{e}"))?;
    let sequence = OCR_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        "{:020}-{:010}-{:06}.json",
        request.created_ms,
        std::process::id(),
        sequence % 1_000_000
    );
    let path = dir.join(name);
    let temporary = path.with_extension("tmp");
    let text = serde_json::to_string(request).map_err(|e| format!("序列化 OCR 请求失败：{e}"))?;
    fs::write(&temporary, text).map_err(|e| format!("写入 OCR 请求失败：{e}"))?;
    fs::rename(&temporary, &path).map_err(|e| format!("提交 OCR 请求失败：{e}"))?;
    Ok(path.with_extension("ack"))
}

fn wait_for_ocr_window_request_ack(path: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if path.is_file() {
            let _ = fs::remove_file(path);
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

fn take_ocr_window_request() -> Option<OcrWindowRequest> {
    let dir = ocr_window_request_dir();
    let mut paths = fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let _ = fs::remove_file(&path);
        let Ok(request) = serde_json::from_str::<OcrWindowRequest>(&text) else {
            continue;
        };
        let age_ms = now_millis().saturating_sub(request.created_ms);
        if request.created_ms == 0 || age_ms > OCR_WINDOW_REQUEST_MAX_AGE_MS {
            runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Basic,
                "ocr_window_request",
                format!("status=ignored reason=stale age_ms={age_ms}"),
            );
            continue;
        }
        let _ = fs::write(path.with_extension("ack"), b"accepted");
        return Some(request);
    }
    None
}

fn capture_session_from_image_file(
    path: &Path,
    target_hwnd: isize,
    delete_source_after_import: bool,
) -> Result<CaptureSession, String> {
    if !path.is_file() {
        return Err(format!("截图文件不存在：{}", path.display()));
    }
    let metadata = fs::metadata(path).map_err(|e| format!("读取截图文件信息失败：{e}"))?;
    if metadata.len() == 0 || metadata.len() > MAX_OCR_IMPORT_BYTES {
        return Err(format!(
            "截图文件大小不合法：{} 字节（上限 {} MiB）。",
            metadata.len(),
            MAX_OCR_IMPORT_BYTES / 1024 / 1024
        ));
    }
    let (width, height) =
        image::image_dimensions(path).map_err(|e| format!("读取截图尺寸失败：{e}"))?;
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_OCR_IMPORT_PIXELS {
        return Err(format!("截图尺寸超过安全限制：{width}×{height}。"));
    }
    let temp_path = temp_ocr_image_path("file");
    // Handoff screenshots may be JPEG/BMP while our private temporary path is
    // always named `.png`. A byte-for-byte copy leaves JPEG data behind a PNG
    // extension; image decoders and RapidOCR can then select the wrong codec,
    // producing an empty preview even though the request was received. Decode
    // from the source bytes and write a real PNG for both preview and OCR.
    let is_png = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("png"));
    let import_result = if is_png {
        fs::copy(path, &temp_path)
            .map(|_| ())
            .map_err(|e| format!("复制 PNG 截图失败：{e}"))
    } else {
        image::open(path)
            .map_err(|e| format!("解码截图文件失败：{e}"))
            .and_then(|image| {
                image
                    .save_with_format(&temp_path, image::ImageFormat::Png)
                    .map_err(|e| format!("转换截图为 PNG 失败：{e}"))
            })
    };
    if let Err(error) = import_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    let save_note = if delete_source_after_import {
        save_ocr_capture_if_enabled(path)
    } else {
        OcrCaptureSaveNote {
            saved_path: Some(path.to_path_buf()),
            error: None,
        }
    };
    if delete_source_after_import && save_note.error.is_none() {
        if let Err(error) = fs::remove_file(path) {
            runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Error,
                "ocr_handoff_cleanup",
                format!(
                    "status=failed path={} reason={error}",
                    runtime_log::path_for_log(path)
                ),
            );
        }
    }
    Ok(CaptureSession {
        source: CaptureSource::ImageFile,
        target_hwnd,
        original_path: temp_path,
        input_path: Some(path.to_path_buf()),
        fallback_reason: None,
        save_note,
        detected_crop: None,
    })
}

fn open_translate_from_ocr(
    text: &str,
    target_hwnd: isize,
    screenshot_path: Option<&Path>,
) -> Result<(), String> {
    let mut request = external_translation::ExternalTranslationRequest::new(text, "ocr");
    request.target_hwnd = (target_hwnd != 0).then_some(target_hwnd);
    request.target_process_id = external_translation::process_id_for_window(target_hwnd);
    request.result_action = external_translation::preferred_result_action();
    request.screenshot_path = screenshot_path.map(Path::to_path_buf);
    // WinTranslator owns both the translation UI and any configured result
    // action, so OCR never creates an input-method result window.
    request.presentation = "full".to_string();
    request.interactive = true;
    request.delivery = request.result_action.clone();
    external_translation::launch_full_request(&request)
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() -> eframe::Result<()> {
    windows_security::apply_process_hardening();
    let startup_request = ocr_window_request_from_args();
    runtime_log::log_ocr(
        runtime_log::RuntimeLogLevel::Basic,
        "ocr_startup_request",
        format!(
            "target_hwnd={} manual_region={} translate={} image={}",
            startup_request.target_hwnd,
            if startup_request.manual_region { 1 } else { 0 },
            if startup_request.translate { 1 } else { 0 },
            startup_request
                .image
                .as_deref()
                .map(runtime_log::path_for_log)
                .unwrap_or_else(|| "(none)".to_string())
        ),
    );
    let _single_instance_guard = match win_single_instance::claim_or_activate_existing(
        OCR_SINGLE_INSTANCE_MUTEX,
        WINDOW_TITLE,
    ) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            match write_ocr_window_request(&startup_request) {
                Ok(ack_path) => {
                    let acknowledged = wait_for_ocr_window_request_ack(&ack_path);
                    runtime_log::log_ocr(
                        runtime_log::RuntimeLogLevel::Basic,
                        "ocr_single_instance_forward",
                        format!(
                            "status={} target_hwnd={} manual_region={} translate={} image={}",
                            if acknowledged {
                                "accepted"
                            } else {
                                "ack_timeout"
                            },
                            startup_request.target_hwnd,
                            if startup_request.manual_region { 1 } else { 0 },
                            if startup_request.translate { 1 } else { 0 },
                            startup_request
                                .image
                                .as_deref()
                                .map(runtime_log::path_for_log)
                                .unwrap_or_else(|| "(none)".to_string())
                        ),
                    );
                }
                Err(err) => runtime_log::log_ocr(
                    runtime_log::RuntimeLogLevel::Error,
                    "ocr_single_instance_forward",
                    format!("status=failed reason={}", log_error_detail(&err)),
                ),
            }
            return Ok(());
        }
        Err(err) => {
            runtime_log::log_ocr(
                runtime_log::RuntimeLogLevel::Error,
                "ocr_single_instance",
                format!("status=mutex_error reason={err}"),
            );
            None
        }
    };

    let target_hwnd = startup_request.target_hwnd;
    let translate_after_ocr = startup_request.translate;
    // Start Python, load the model and execute one tiny graph while the user
    // is selecting/capturing the screen region. The recognition worker shares
    // the same process slot and will simply wait if capture completes first.
    start_rapidocr_prewarm(
        OcrLanguageMode::from_config(&tool_prefs::ocr_language()),
        ocr_speed_profile(),
    );
    let startup_capture = if let Some(image) = startup_request.image.as_ref() {
        match capture_session_from_image_file(
            image,
            target_hwnd,
            startup_request.delete_source_after_import,
        ) {
            Ok(session) => StartupCapture::Captured(session),
            Err(err) => StartupCapture::Failed(err),
        }
    } else if startup_request.manual_region {
        StartupCapture::ManualRegion
    } else {
        match capture_screenshot_image(target_hwnd, CaptureMode::AutoWindow) {
            Ok(capture) => {
                let save_note = save_ocr_capture_if_enabled(&capture.path);
                StartupCapture::Captured(CaptureSession {
                    source: capture.source,
                    target_hwnd,
                    original_path: capture.path,
                    input_path: None,
                    fallback_reason: capture.fallback_reason,
                    save_note,
                    detected_crop: None,
                })
            }
            Err(err) => StartupCapture::Failed(err),
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(DEFAULT_WINDOW_SIZE)
            .with_min_inner_size(MIN_WINDOW_SIZE)
            .with_always_on_top()
            .with_title(WINDOW_TITLE),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(|cc| {
            Ok(Box::new(OcrApp::new(
                cc,
                target_hwnd,
                translate_after_ocr,
                startup_capture,
            )))
        }),
    )
}
