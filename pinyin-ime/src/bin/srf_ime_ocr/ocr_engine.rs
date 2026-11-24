//! Native in-process RapidOCR engine service + optional headless `--ocr-worker` CLI.
//!
//! Replaces `tools/kaixin_ocr_engine.py` by calling `rapidocrlite.dll` directly via FFI.

use crate::border_crop::detect_window_border;
use crate::ocr_policy::{
    analyze_image, image_tiles, intersection, line_bounds, lines_in_region, needs_detector_retry,
    rect_area, refinement_regions, should_retry_preprocess, ImageAnalysisSummary, PolicyLine,
    PolicyProfile, Rect, WorkBudget,
};
use crate::rapidocr_ffi::{
    NativeRapidOcr, RapidOcrDll, RpocrOptions, RPOCR_BACKEND_CPU, RPOCR_MODEL_PPOCR,
    RPOCR_TASK_DET, RPOCR_TASK_REC,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use image::{imageops, DynamicImage, Rgb, RgbImage};
use pinyin_ime::rapidocr_paths;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const MAX_ENCODED_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECODED_IMAGE_BYTES: usize = 48 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_IMAGE_SIDE: u32 = 16_384;

const DET_THRESH: f32 = 0.3;
const DET_BOX_THRESH: f32 = 0.5;
const DET_UNCLIP_RATIO: f32 = 1.6;
const DET_MAX_CANDIDATES: i32 = 1000;
const REC_HEIGHT: i32 = 48;
const WIDTH_HEIGHT_RATIO: f64 = 8.0;
const MIN_HEIGHT_PAD: f64 = 30.0;

#[derive(Clone, Debug)]
pub struct NativeOcrConfig {
    pub rapidocr_root: PathBuf,
    pub model_root: PathBuf,
    pub intra_op_threads: usize,
    pub rec_batch_num: usize,
    pub cache_limit: usize,
    pub fast_mode: bool,
}

pub fn default_intra_op_threads() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    cpus.saturating_sub(2).clamp(1, 4)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_bool(name: &str, default: bool) -> bool {
    let Ok(value) = std::env::var(name) else {
        return default;
    };
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no" | "disabled"
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct EngineKey {
    det_variant: String,
    rec_variant: String,
    limit: u32,
    rec_batch_num: usize,
    fast_mode: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelFileStat {
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct OcrAttemptLog {
    pub kind: String,
    pub language: &'static str,
    pub retry: usize,
    pub ok: bool,
    pub detector: String,
    pub recognizer: String,
    pub rect: [i32; 4],
    pub scale: f64,
    pub pixels: u64,
    pub init_ms: f64,
    pub infer_ms: f64,
    pub total_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct OcrOutputLine {
    pub index: usize,
    pub text: String,
    pub score: f32,
    #[serde(rename = "box")]
    pub box_pts: Option<[[f32; 2]; 4]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WarmupResponse {
    pub ok: bool,
    pub command: &'static str,
    pub cached: bool,
    pub profile: &'static str,
    pub detector_variant: &'static str,
    pub recognizer_variant: &'static str,
    pub already_warm: bool,
    pub warmed_detection: bool,
    pub warmed_recognition: bool,
    pub cached_engines: usize,
    pub init_ms: f64,
    pub warmup_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecognizeResponse {
    pub ok: bool,
    pub engine: &'static str,
    pub backend: &'static str,
    pub profile: &'static str,
    pub ocr_version: &'static str,
    pub model_type: String,
    pub detector_variant: &'static str,
    pub recognizer_variant: &'static str,
    pub fast_mode: bool,
    pub max_side_len: u32,
    pub min_side_len: f64,
    pub language: String,
    pub model_language: &'static str,
    pub auto_language_retry: bool,
    pub auto_preprocess_retry: bool,
    pub detector_fallback: bool,
    pub image: String,
    pub rapidocr_root: String,
    pub model_root: String,
    pub models: Vec<ModelFileStat>,
    pub cached: bool,
    pub cached_engines: usize,
    pub init_ms: f64,
    pub load_ms: f64,
    pub infer_ms: f64,
    pub preprocess_ms: f64,
    pub total_ms: f64,
    pub attempts: Vec<OcrAttemptLog>,
    pub tile_count: usize,
    pub analysis: ImageAnalysisSummary,
    pub work_pixels: u64,
    pub optional_calls: usize,
    pub skipped_passes: Vec<String>,
    pub step_ms: Vec<f64>,
    pub text: String,
    pub lines: Vec<OcrOutputLine>,
    pub low_confidence_lines: Vec<OcrOutputLine>,
    pub vis: Option<String>,
}

pub enum ImageInput<'a> {
    Path(&'a Path),
    Bytes { bytes: &'a [u8], label: String },
}

pub struct NativeOcrService {
    config: NativeOcrConfig,
    dll: Arc<RapidOcrDll>,
    // LRU order: front = least recently used, back = most recently used.
    engines: Vec<(EngineKey, NativeRapidOcr)>,
    warmed: HashSet<EngineKey>,
}

impl NativeOcrService {
    pub fn new(config: NativeOcrConfig) -> Result<Self, String> {
        if std::env::var_os("PPOCR_BENCH_THREADS").is_none() {
            std::env::set_var("PPOCR_BENCH_THREADS", config.intra_op_threads.to_string());
        }
        let missing = missing_required_models(&config.model_root);
        if !missing.is_empty() {
            let joined = missing
                .into_iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "required RapidOCR model files are missing: {joined}"
            ));
        }
        let dll_path = rapidocr_paths::rapidocr_dll_path(&config.rapidocr_root);
        let dll = RapidOcrDll::load(&dll_path)?;
        Ok(Self {
            config,
            dll,
            engines: Vec::new(),
            warmed: HashSet::new(),
        })
    }

    pub fn close(&mut self) {
        self.engines.clear();
        self.warmed.clear();
    }

    fn engine_key(
        &self,
        profile: PolicyProfile,
        detector_override: Option<&str>,
        limit_override: Option<u32>,
    ) -> EngineKey {
        let (default_det, default_rec) = profile.models();
        let det = detector_override.unwrap_or(default_det);
        let limit = limit_override.unwrap_or_else(|| profile.limits().0);
        EngineKey {
            det_variant: det.to_string(),
            rec_variant: default_rec.to_string(),
            limit,
            rec_batch_num: self.config.rec_batch_num,
            fast_mode: self.config.fast_mode && profile == PolicyProfile::Fast,
        }
    }

    fn build_options(&self, key: &EngineKey) -> RpocrOptions {
        let mut opts = self.dll.default_options();
        opts.backend = RPOCR_BACKEND_CPU;
        opts.model_type = RPOCR_MODEL_PPOCR;
        opts.det_threshold = DET_THRESH;
        opts.det_box_threshold = DET_BOX_THRESH;
        opts.det_unclip_ratio = DET_UNCLIP_RATIO;
        opts.det_limit_side_len = key.limit as i32;
        opts.det_limit_type = 0; // max
        opts.det_mean = [0.5, 0.5, 0.5];
        opts.det_std = [0.5, 0.5, 0.5];
        opts.rec_mean = [0.5, 0.5, 0.5];
        opts.rec_std = [0.5, 0.5, 0.5];
        opts.det_use_dilation = 1;
        opts.det_max_candidates = DET_MAX_CANDIDATES;
        opts.rec_height = REC_HEIGHT;
        opts.rec_batch_size = key.rec_batch_num.max(1) as i32;
        opts.rec_fast_mode = if key.fast_mode { 1 } else { 0 };
        opts.use_cls = 0;
        opts.cls_model = std::ptr::null();
        opts
    }

    fn ensure_engine(&mut self, key: &EngineKey) -> Result<(usize, f64, bool), String> {
        if let Some(pos) = self.engines.iter().position(|(k, _)| k == key) {
            let entry = self.engines.remove(pos);
            self.engines.push(entry);
            return Ok((self.engines.len() - 1, 0.0, true));
        }

        let limit = self.config.cache_limit.clamp(1, 2);
        while self.engines.len() >= limit {
            let (evicted_key, evicted_engine) = self.engines.remove(0);
            self.warmed.remove(&evicted_key);
            drop(evicted_engine);
        }

        let det_path = self
            .config
            .model_root
            .join(format!("PP-OCRv6_det_{}.onnx", key.det_variant));
        let rec_path = self
            .config
            .model_root
            .join(format!("PP-OCRv6_rec_{}.onnx", key.rec_variant));
        let dict_path = self.config.model_root.join("ppocrv6_dict.txt");
        let opts = self.build_options(key);

        let started = Instant::now();
        let mut engine = NativeRapidOcr::create(
            Arc::clone(&self.dll),
            &det_path,
            &rec_path,
            &dict_path,
            &opts,
        )?;
        engine.set_det_thresholds(DET_THRESH, DET_BOX_THRESH, DET_UNCLIP_RATIO)?;
        let elapsed = started.elapsed().as_secs_f64();
        self.engines.push((key.clone(), engine));
        Ok((self.engines.len() - 1, elapsed, false))
    }

    fn clear_engine(&mut self, key: &EngineKey) {
        if let Some(pos) = self.engines.iter().position(|(k, _)| k == key) {
            let (_, evicted) = self.engines.remove(pos);
            drop(evicted);
        }
        self.warmed.remove(key);
    }

    pub fn warmup(&mut self, profile: PolicyProfile) -> Result<WarmupResponse, String> {
        let key = self.engine_key(profile, None, None);
        let (idx, init_seconds, cached) = self.ensure_engine(&key)?;
        let warm_started = Instant::now();
        let already_warm = self.warmed.contains(&key);

        if !already_warm {
            let width = profile.limits().0;
            let height = (width * 9 / 16).max(64);
            let sample = build_warmup_sample(width, height);
            let top = (height / 2).saturating_sub(40);
            let left = (width / 12).saturating_sub(8);
            let strip_w = 328u32.min(width.saturating_sub(left)).max(32);
            let strip_h = 48u32.min(height.saturating_sub(top)).max(16);
            let text_line = crop_rgb(&sample, left, top, strip_w, strip_h);

            let (det_img, _, _, _) = prepare_rapidocr_input(
                &self.dll,
                &sample,
                0.0,
                f64::from(profile.limits().0),
                true,
            )?;
            let (rec_img, _, _, _) = prepare_rapidocr_input(
                &self.dll,
                &text_line,
                0.0,
                f64::from(profile.limits().0),
                false,
            )?;

            let run_result = (|| -> Result<(), String> {
                let engine = &mut self.engines[idx].1;
                engine.run_rgb(
                    det_img.as_raw(),
                    det_img.width(),
                    det_img.height(),
                    RPOCR_TASK_DET,
                )?;
                engine.run_rgb(
                    rec_img.as_raw(),
                    rec_img.width(),
                    rec_img.height(),
                    RPOCR_TASK_REC,
                )?;
                Ok(())
            })();

            if let Err(err) = run_result {
                self.clear_engine(&key);
                return Err(err);
            }
            self.warmed.insert(key);
        }

        let (det_variant, rec_variant) = profile.models();
        Ok(WarmupResponse {
            ok: true,
            command: "warmup",
            cached,
            profile: profile.as_str(),
            detector_variant: det_variant,
            recognizer_variant: rec_variant,
            already_warm,
            warmed_detection: true,
            warmed_recognition: true,
            cached_engines: self.engines.len(),
            init_ms: round2(init_seconds * 1000.0),
            warmup_ms: round2(warm_started.elapsed().as_secs_f64() * 1000.0),
        })
    }

    pub fn recognize(
        &mut self,
        input: ImageInput<'_>,
        lang: &str,
        min_score: f32,
        profile: PolicyProfile,
        max_side_override: Option<u32>,
        vis_path: Option<&Path>,
    ) -> Result<RecognizeResponse, String> {
        let request_start = Instant::now();
        if !matches!(lang, "auto" | "mixed" | "zh" | "zh_tw" | "en") {
            return Err(format!("unsupported language: {lang:?}"));
        }

        let load_start = Instant::now();
        let (image_rgb, image_label) = match input {
            ImageInput::Path(path) => {
                if !path.is_file() {
                    return Err(format!("image not found: {}", path.display()));
                }
                let decoded = image::open(path)
                    .map_err(|err| format!("failed to decode image {}: {err}", path.display()))?;
                (decoded.to_rgb8(), path.display().to_string())
            }
            ImageInput::Bytes { bytes, label } => {
                if bytes.len() > MAX_DECODED_IMAGE_BYTES {
                    return Err(format!(
                        "decoded image is too large: {} bytes (limit {MAX_DECODED_IMAGE_BYTES})",
                        bytes.len()
                    ));
                }
                let decoded = image::load_from_memory(bytes)
                    .map_err(|err| format!("failed to decode memory image: {err}"))?;
                (decoded.to_rgb8(), label)
            }
        };
        validate_image_dimensions(image_rgb.width(), image_rgb.height())?;
        let load_seconds = load_start.elapsed().as_secs_f64();

        let (applied_max_side, applied_min_side) = adaptive_limits(
            profile,
            image_rgb.width(),
            image_rgb.height(),
            max_side_override,
        );
        let tiles = image_tiles(image_rgb.width(), image_rgb.height(), applied_max_side)?;
        let mut budget = WorkBudget::new(profile.budget_seconds());

        let analysis_start = Instant::now();
        let analysis = analyze_image(&image_rgb);
        let mut preprocess_seconds = analysis_start.elapsed().as_secs_f64();

        let mut attempts: Vec<OcrAttemptLog> = Vec::new();
        let mut total_init = 0.0f64;
        let mut total_infer = 0.0f64;
        let mut all_cached = true;
        let (primary_det, primary_rec) = profile.models();
        let mut step_seconds: Vec<f64> = vec![0.0, 0.0, 0.0];

        let max_attempts = 2usize;

        let mut run_region = |svc: &mut Self,
                              budget: &mut WorkBudget,
                              region: Rect,
                              kind: &str,
                              scale: f64,
                              detector: Option<&str>,
                              enhance: bool|
         -> Result<Option<Vec<PolicyLine>>, String> {
            let optional = kind != "primary";
            let [x0, y0, x1, y1] = region;
            let region_w = (x1 - x0).max(1) as f64;
            let region_h = (y1 - y0).max(1) as f64;
            let bounded_scale = scale.min(f64::from(applied_max_side) / region_w.max(region_h));
            let target_w = (region_w * bounded_scale).round().max(1.0) as u32;
            let target_h = (region_h * bounded_scale).round().max(1.0) as u32;
            let pixels = u64::from(target_w) * u64::from(target_h);

            let mut last_error: Option<String> = None;
            for attempt in 1..=max_attempts {
                if !budget.reserve(pixels, kind, optional)? {
                    return Ok(None);
                }

                let attempt_start = Instant::now();
                let mut attempt_init = 0.0f64;
                let mut attempt_infer = 0.0f64;
                let mut succeeded = false;
                let key = svc.engine_key(profile, detector, Some(applied_max_side));

                let step_result = (|| -> Result<Vec<PolicyLine>, String> {
                    let (idx, init_seconds, cached) = svc.ensure_engine(&key)?;
                    total_init += init_seconds;
                    all_cached = all_cached && cached;
                    attempt_init = init_seconds;

                    let prepare_start = Instant::now();
                    let mut source = crop_rgb(
                        &image_rgb,
                        x0 as u32,
                        y0 as u32,
                        (x1 - x0) as u32,
                        (y1 - y0) as u32,
                    );
                    if source.width() != target_w || source.height() != target_h {
                        source = imageops::resize(
                            &source,
                            target_w,
                            target_h,
                            imageops::FilterType::CatmullRom,
                        );
                    }
                    if enhance {
                        source = enhance_contrast_clahe(&source);
                    }
                    let (prepared, ratio_w, ratio_h, pad_top) = prepare_rapidocr_input(
                        &svc.dll,
                        &source,
                        0.0,
                        f64::from(applied_max_side),
                        true,
                    )?;
                    preprocess_seconds += prepare_start.elapsed().as_secs_f64();

                    let infer_start = Instant::now();
                    let raw_boxes = svc.engines[idx].1.run_rgb(
                        prepared.as_raw(),
                        prepared.width(),
                        prepared.height(),
                        RPOCR_TASK_DET | RPOCR_TASK_REC,
                    )?;
                    attempt_infer = infer_start.elapsed().as_secs_f64();
                    succeeded = true;
                    step_seconds[2] += attempt_infer;

                    let sx = f64::from(target_w) / region_w;
                    let sy = f64::from(target_h) / region_h;
                    let img_w = image_rgb.width() as f64;
                    let img_h = image_rgb.height() as f64;
                    let mut mapped = Vec::with_capacity(raw_boxes.len());

                    for (line_idx, raw) in raw_boxes.into_iter().enumerate() {
                        let unpadded = map_quad_to_original(
                            raw.quad,
                            ratio_w,
                            ratio_h,
                            pad_top,
                            source.width() as f32,
                            source.height() as f32,
                        );
                        let min_bx = unpadded
                            .iter()
                            .map(|p| f64::from(p[0]))
                            .fold(f64::INFINITY, f64::min);
                        let max_bx = unpadded
                            .iter()
                            .map(|p| f64::from(p[0]))
                            .fold(f64::NEG_INFINITY, f64::max);
                        let min_by = unpadded
                            .iter()
                            .map(|p| f64::from(p[1]))
                            .fold(f64::INFINITY, f64::min);
                        let max_by = unpadded
                            .iter()
                            .map(|p| f64::from(p[1]))
                            .fold(f64::NEG_INFINITY, f64::max);

                        let clip_left = x0 > 0 && min_bx < 4.0 * sx;
                        let clip_right =
                            (x1 as f64) < img_w && max_bx > f64::from(target_w) - 4.0 * sx;
                        let tile_edge = clip_left
                            || (y0 > 0 && min_by < 4.0 * sy)
                            || clip_right
                            || ((y1 as f64) < img_h && max_by > f64::from(target_h) - 4.0 * sy);

                        let mut mapped_quad = [[0.0f32; 2]; 4];
                        for (k, p) in unpadded.iter().enumerate() {
                            let gx = (f64::from(p[0]) / sx + f64::from(x0))
                                .clamp(f64::from(x0), f64::from(x1));
                            let gy = (f64::from(p[1]) / sy + f64::from(y0))
                                .clamp(f64::from(y0), f64::from(y1));
                            mapped_quad[k] = [gx as f32, gy as f32];
                        }

                        mapped.push(PolicyLine {
                            index: line_idx,
                            text: raw.text,
                            score: round6_f32(raw.score),
                            box_pts: Some(mapped_quad),
                            tile_edge,
                            clip_left,
                            clip_right,
                        });
                    }
                    Ok(mapped)
                })();

                total_infer += attempt_infer;
                attempts.push(OcrAttemptLog {
                    kind: kind.to_string(),
                    language: "multilingual",
                    retry: attempt,
                    ok: succeeded,
                    detector: detector.unwrap_or(primary_det).to_string(),
                    recognizer: primary_rec.to_string(),
                    rect: region,
                    scale: round3(bounded_scale),
                    pixels,
                    init_ms: round2(attempt_init * 1000.0),
                    infer_ms: round2(attempt_infer * 1000.0),
                    total_ms: round2(attempt_start.elapsed().as_secs_f64() * 1000.0),
                });

                match step_result {
                    Ok(lines) => return Ok(Some(lines)),
                    Err(err) => {
                        svc.clear_engine(&key);
                        last_error = Some(err);
                    }
                }
            }

            if optional {
                budget.skipped.push(format!("{kind}:inference-error"));
                return Ok(None);
            }
            Err(last_error.unwrap_or_else(|| "RapidOCR inference failed".to_string()))
        };

        let mut tile_lines: Vec<PolicyLine> = Vec::new();
        let short_side = image_rgb.width().min(image_rgb.height()).max(1);
        let tiny_scale = (applied_min_side / f64::from(short_side)).max(1.0);
        for &tile in &tiles {
            if let Some(found) =
                run_region(self, &mut budget, tile, "primary", tiny_scale, None, false)?
            {
                tile_lines.extend(found);
            }
        }
        let mut lines = merge_tile_lines(tile_lines);

        let upscale = f64::from(profile.limits().1);
        if upscale > 1.0 {
            for region in refinement_regions(&lines, &analysis) {
                let max_span = f64::from((region[2] - region[0]).max(region[3] - region[1]));
                if max_span * upscale > f64::from(applied_max_side) {
                    continue;
                }
                if let Some(candidate) = run_region(
                    self,
                    &mut budget,
                    region,
                    "small-text",
                    upscale,
                    None,
                    false,
                )? {
                    accept_region(&mut lines, region, candidate);
                }
            }
        }

        if should_retry_preprocess(&lines, &analysis) {
            for &region in analysis.regions.iter().take(8) {
                let previous = lines_in_region(&lines, region);
                if !previous.is_empty() && recognition_avg_score(&previous) >= 0.82 {
                    continue;
                }
                if let Some(candidate) =
                    run_region(self, &mut budget, region, "contrast", 1.0, None, true)?
                {
                    accept_region(&mut lines, region, candidate);
                }
            }
        }

        if profile == PolicyProfile::Balanced && needs_detector_retry(&lines, &analysis) {
            for &region in &analysis.regions {
                if lines_in_region(&lines, region).is_empty() {
                    if let Some(candidate) = run_region(
                        self,
                        &mut budget,
                        region,
                        "detector-fallback",
                        1.0,
                        Some("medium"),
                        false,
                    )? {
                        accept_region(&mut lines, region, candidate);
                    }
                }
                if budget.extra_calls >= 8 {
                    break;
                }
            }
        }

        let vis_status = if let Some(path) = vis_path {
            save_visualization(&image_rgb, &lines, path)?;
            Some(path.display().to_string())
        } else {
            None
        };

        let mut kept_lines = Vec::new();
        let mut low_confidence_lines = Vec::new();
        for line in lines {
            let item = OcrOutputLine {
                index: line.index,
                text: line.text,
                score: line.score,
                box_pts: line.box_pts,
            };
            if item.score >= min_score {
                kept_lines.push(item);
            } else {
                low_confidence_lines.push(item);
            }
        }
        let text = kept_lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let auto_preprocess_retry = attempts.iter().any(|item| item.kind == "contrast");
        let detector_fallback = attempts.iter().any(|item| item.kind == "detector-fallback");

        Ok(RecognizeResponse {
            ok: true,
            engine: "rapidocr",
            backend: "rapidocrlite/cpu",
            profile: profile.as_str(),
            ocr_version: "PP-OCRv6",
            model_type: format!("{primary_det}-det+{primary_rec}-rec"),
            detector_variant: primary_det,
            recognizer_variant: primary_rec,
            fast_mode: self.config.fast_mode && profile == PolicyProfile::Fast,
            max_side_len: applied_max_side,
            min_side_len: applied_min_side,
            language: if lang == "auto" {
                "mixed".to_string()
            } else {
                lang.to_string()
            },
            model_language: "multilingual",
            auto_language_retry: false,
            auto_preprocess_retry,
            detector_fallback,
            image: image_label,
            rapidocr_root: self.config.rapidocr_root.display().to_string(),
            model_root: self.config.model_root.display().to_string(),
            models: list_model_files(&self.config.model_root),
            cached: all_cached,
            cached_engines: self.engines.len(),
            init_ms: round2(total_init * 1000.0),
            load_ms: round2(load_seconds * 1000.0),
            infer_ms: round2(total_infer * 1000.0),
            preprocess_ms: round2(preprocess_seconds * 1000.0),
            total_ms: round2(request_start.elapsed().as_secs_f64() * 1000.0),
            attempts,
            tile_count: tiles.len(),
            analysis: analysis.summary(),
            work_pixels: budget.pixels,
            optional_calls: budget.extra_calls,
            skipped_passes: budget.skipped,
            step_ms: step_seconds
                .into_iter()
                .map(|s| round2(s * 1000.0))
                .collect(),
            text,
            lines: kept_lines,
            low_confidence_lines,
            vis: vis_status,
        })
    }
}

pub fn adaptive_limits(
    profile: PolicyProfile,
    width: u32,
    height: u32,
    max_side_override: Option<u32>,
) -> (u32, f64) {
    let (default_max, max_upscale) = profile.limits();
    let max_side_len = max_side_override
        .map(|v| v.clamp(320, 4096))
        .unwrap_or(default_max);
    let short_side = width.min(height).max(1);
    let min_side_len = if short_side >= 20 || max_upscale <= 1.0 {
        0.0
    } else {
        (f64::from(short_side) * f64::from(max_upscale)).min(30.0)
    };
    (max_side_len, min_side_len)
}

fn validate_image_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err(format!("image has invalid dimensions: {width}x{height}"));
    }
    let pixels = u64::from(width) * u64::from(height);
    if width.max(height) > MAX_IMAGE_SIDE || pixels > MAX_IMAGE_PIXELS {
        return Err(format!(
            "image dimensions exceed safety limit: {width}x{height} (max side {MAX_IMAGE_SIDE}, max pixels {MAX_IMAGE_PIXELS})"
        ));
    }
    Ok(())
}

fn missing_required_models(model_root: &Path) -> Vec<PathBuf> {
    rapidocr_paths::REQUIRED_ONNX_MODELS
        .iter()
        .map(|name| model_root.join(name))
        .filter(|path| !path.is_file())
        .collect()
}

fn list_model_files(model_root: &Path) -> Vec<ModelFileStat> {
    let Ok(entries) = std::fs::read_dir(model_root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("onnx"))
        {
            if let Ok(meta) = path.metadata() {
                out.push(ModelFileStat {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    bytes: meta.len(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn prepare_rapidocr_input(
    dll: &RapidOcrDll,
    original: &RgbImage,
    min_side_len: f64,
    max_side_len: f64,
    pad: bool,
) -> Result<(RgbImage, f32, f32, i32), String> {
    let (mut img, ratio_h, ratio_w) =
        resize_image_within_bounds(dll, original, min_side_len, max_side_len)?;
    let pad_top = if pad {
        let (padded, top) = apply_vertical_padding(img, WIDTH_HEIGHT_RATIO, MIN_HEIGHT_PAD);
        img = padded;
        top
    } else {
        0
    };
    Ok((img, ratio_w, ratio_h, pad_top))
}

fn resize_image_within_bounds(
    dll: &RapidOcrDll,
    img: &RgbImage,
    min_side_len: f64,
    max_side_len: f64,
) -> Result<(RgbImage, f32, f32), String> {
    let mut current = img.clone();
    let mut ratio_h = 1.0f32;
    let mut ratio_w = 1.0f32;
    if f64::from(current.width().max(current.height())) > max_side_len {
        let (next, rh, rw) = resize_to_ratio(dll, &current, max_side_len, false)?;
        current = next;
        ratio_h = rh;
        ratio_w = rw;
    }
    if min_side_len > 0.0 && f64::from(current.width().min(current.height())) < min_side_len {
        let (next, rh, rw) = resize_to_ratio(dll, &current, min_side_len, true)?;
        current = next;
        ratio_h = rh;
        ratio_w = rw;
    }
    Ok((current, ratio_h, ratio_w))
}

fn resize_to_ratio(
    dll: &RapidOcrDll,
    img: &RgbImage,
    side: f64,
    grow: bool,
) -> Result<(RgbImage, f32, f32), String> {
    let width = img.width();
    let height = img.height();
    let mut ratio = 1.0f64;
    if grow {
        let min_s = width.min(height);
        if f64::from(min_s) < side {
            ratio = side / f64::from(min_s);
        }
    } else {
        let max_s = width.max(height);
        if f64::from(max_s) > side {
            ratio = side / f64::from(max_s);
        }
    }
    let resize_h = (((f64::from(height) * ratio).trunc() / 32.0).round() * 32.0) as u32;
    let resize_w = (((f64::from(width) * ratio).trunc() / 32.0).round() * 32.0) as u32;
    if resize_w == 0 || resize_h == 0 {
        return Err("resize_w or resize_h is less than or equal to 0".to_string());
    }
    if resize_h == height && resize_w == width {
        return Ok((img.clone(), 1.0, 1.0));
    }

    let resized =
        if let Some(buf) = dll.resize_linear_rgb(img.as_raw(), width, height, resize_w, resize_h) {
            RgbImage::from_raw(resize_w, resize_h, buf)
                .ok_or_else(|| "invalid native resized RGB buffer".to_string())?
        } else {
            imageops::resize(img, resize_w, resize_h, imageops::FilterType::Triangle)
        };
    Ok((
        resized,
        height as f32 / resize_h as f32,
        width as f32 / resize_w as f32,
    ))
}

fn apply_vertical_padding(
    img: RgbImage,
    width_height_ratio: f64,
    min_height: f64,
) -> (RgbImage, i32) {
    let width = img.width();
    let height = img.height();
    let use_limit_ratio = if width_height_ratio == -1.0 {
        false
    } else {
        f64::from(width) / f64::from(height.max(1)) > width_height_ratio
    };
    if f64::from(height) <= min_height || use_limit_ratio {
        let new_h = ((f64::from(width) / width_height_ratio) as i32).max(min_height as i32) * 2;
        let padding_h = (new_h - height as i32).abs() / 2;
        if padding_h <= 0 {
            return (img, 0);
        }
        let pad = padding_h as u32;
        let padded_h = height + pad * 2;
        let row_bytes = (width as usize) * 3;
        let mut raw = vec![0u8; (padded_h as usize) * row_bytes];
        let src = img.as_raw();
        let offset = (pad as usize) * row_bytes;
        raw[offset..offset + src.len()].copy_from_slice(src);
        let padded = RgbImage::from_raw(width, padded_h, raw).expect("valid padded RGB image");
        return (padded, padding_h);
    }
    (img, 0)
}

fn map_quad_to_original(
    quad: [[f32; 2]; 4],
    ratio_w: f32,
    ratio_h: f32,
    pad_top: i32,
    ori_w: f32,
    ori_h: f32,
) -> [[f32; 2]; 4] {
    let mut out = [[0.0f32; 2]; 4];
    let pad = pad_top as f32;
    for (i, [x, y]) in quad.into_iter().enumerate() {
        out[i] = [
            (x * ratio_w).clamp(0.0, ori_w),
            ((y - pad) * ratio_h).clamp(0.0, ori_h),
        ];
    }
    out
}

fn crop_rgb(image: &RgbImage, x: u32, y: u32, width: u32, height: u32) -> RgbImage {
    imageops::crop_imm(image, x, y, width, height).to_image()
}

fn build_warmup_sample(width: u32, height: u32) -> RgbImage {
    let mut img = RgbImage::from_pixel(width, height, Rgb([255, 255, 255]));
    let left = width / 12;
    for top in [height / 5, height / 2, height * 4 / 5] {
        // Draw crisp synthetic glyph strokes so both detector and recognizer execute real paths.
        for glyph in 0..14u32 {
            let gx = left + glyph * 22;
            if gx + 16 >= width {
                break;
            }
            let gy = top.saturating_sub(28);
            for dy in 0..24u32 {
                if gy + dy >= height {
                    break;
                }
                for dx in 0..14u32 {
                    let stroke = !(3..11).contains(&dx)
                        || !(3..21).contains(&dy)
                        || (glyph % 2 == 0 && (10..13).contains(&dy));
                    if stroke {
                        img.put_pixel(gx + dx, gy + dy, Rgb([0, 0, 0]));
                    }
                }
            }
        }
    }
    img
}

/// Pure-Rust 8x8 tile CLAHE contrast enhancement on luminance, matching the
/// low-contrast retry behavior previously done via `cv2.createCLAHE(clipLimit=2.0, (8, 8))`.
pub fn enhance_contrast_clahe(image: &RgbImage) -> RgbImage {
    let w = image.width() as usize;
    let h = image.height() as usize;
    if w < 8 || h < 8 {
        return image.clone();
    }

    let raw = image.as_raw();
    let mut luma = vec![0u8; w * h];
    for (i, px) in raw.as_chunks::<3>().0.iter().enumerate() {
        luma[i] = ((77u32 * u32::from(px[0])
            + 150u32 * u32::from(px[1])
            + 29u32 * u32::from(px[2])
            + 128)
            >> 8) as u8;
    }

    let tiles_x = 8usize;
    let tiles_y = 8usize;
    let tile_w = (w as f64) / (tiles_x as f64);
    let tile_h = (h as f64) / (tiles_y as f64);
    let mut luts = vec![[0u8; 256]; tiles_x * tiles_y];

    for ty in 0..tiles_y {
        let y0 = ((ty as f64) * tile_h).floor() as usize;
        let y1 = (((ty + 1) as f64) * tile_h).ceil().min(h as f64) as usize;
        for tx in 0..tiles_x {
            let x0 = ((tx as f64) * tile_w).floor() as usize;
            let x1 = (((tx + 1) as f64) * tile_w).ceil().min(w as f64) as usize;
            let mut hist = [0u32; 256];
            let mut count = 0u32;
            for y in y0..y1 {
                let row = y * w;
                for x in x0..x1 {
                    hist[usize::from(luma[row + x])] += 1;
                    count += 1;
                }
            }
            if count == 0 {
                for (i, slot) in luts[ty * tiles_x + tx].iter_mut().enumerate() {
                    *slot = i as u8;
                }
                continue;
            }
            let clip_limit = ((2.0 * f64::from(count) / 256.0).round() as u32).max(2);
            let mut excess = 0u32;
            for bin in &mut hist {
                if *bin > clip_limit {
                    excess += *bin - clip_limit;
                    *bin = clip_limit;
                }
            }
            let add_all = excess / 256;
            let rem = (excess % 256) as usize;
            if add_all > 0 {
                for bin in &mut hist {
                    *bin += add_all;
                }
            }
            if let Some(step) = 256usize.checked_div(rem) {
                let mut i = 0usize;
                let mut left = rem;
                while left > 0 && i < 256 {
                    hist[i] += 1;
                    left -= 1;
                    i += step;
                }
            }
            let mut cdf = 0u32;
            let scale = 255.0 / f64::from(count);
            for (i, &bin) in hist.iter().enumerate() {
                cdf += bin;
                luts[ty * tiles_x + tx][i] =
                    (f64::from(cdf) * scale).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    let mut out = vec![0u8; raw.len()];
    for y in 0..h {
        let fy = ((y as f64) + 0.5) / tile_h - 0.5;
        let ty0 = fy.floor().clamp(0.0, (tiles_y - 1) as f64) as usize;
        let ty1 = (ty0 + 1).min(tiles_y - 1);
        let wy = (fy - ty0 as f64).clamp(0.0, 1.0);
        for x in 0..w {
            let fx = ((x as f64) + 0.5) / tile_w - 0.5;
            let tx0 = fx.floor().clamp(0.0, (tiles_x - 1) as f64) as usize;
            let tx1 = (tx0 + 1).min(tiles_x - 1);
            let wx = (fx - tx0 as f64).clamp(0.0, 1.0);

            let idx = y * w + x;
            let v = usize::from(luma[idx]);
            let c00 = f64::from(luts[ty0 * tiles_x + tx0][v]);
            let c10 = f64::from(luts[ty0 * tiles_x + tx1][v]);
            let c01 = f64::from(luts[ty1 * tiles_x + tx0][v]);
            let c11 = f64::from(luts[ty1 * tiles_x + tx1][v]);
            let top = c00 * (1.0 - wx) + c10 * wx;
            let bot = c01 * (1.0 - wx) + c11 * wx;
            let new_l = top * (1.0 - wy) + bot * wy;
            let delta = new_l - f64::from(luma[idx]);

            let off = idx * 3;
            for c in 0..3 {
                out[off + c] = (f64::from(raw[off + c]) + delta).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    RgbImage::from_raw(image.width(), image.height(), out).expect("valid CLAHE RGB image")
}

fn recognition_avg_score(lines: &[&PolicyLine]) -> f64 {
    if lines.is_empty() {
        return 0.0;
    }
    lines.iter().map(|line| f64::from(line.score)).sum::<f64>() / lines.len() as f64
}

pub fn recognition_quality(lines: &[PolicyLine]) -> (f64, f64) {
    if lines.is_empty() {
        return (0.0, 0.0);
    }
    let mut evidence = 0.0f64;
    let mut coverage = 0.0f64;
    let mut seen: HashSet<String> = HashSet::new();

    for line in lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        let chars: Vec<char> = text.chars().collect();
        let score = f64::from(line.score).clamp(0.0, 1.0);
        let useful =
            chars.iter().filter(|c| c.is_alphanumeric()).count() as f64 / chars.len() as f64;
        let repeats = chars
            .windows(3)
            .filter(|w| w[0] == w[1] && w[1] == w[2])
            .count();
        let repeat_penalty =
            1.0 - ((repeats as f64) / (chars.len().saturating_sub(2).max(1) as f64)).min(0.75);
        let duplicate_weight = if seen.contains(text) { 0.5 } else { 1.0 };
        seen.insert(text.to_string());
        let weight = score.powi(3) * (0.5 + useful * 0.5) * repeat_penalty * duplicate_weight;
        evidence += (chars.len() as f64).ln_1p() * weight;
        if let Some(quad) = line.box_pts {
            let mut shoelace = 0.0f64;
            for i in 0..4 {
                let next = (i + 1) % 4;
                shoelace += f64::from(quad[i][0]) * f64::from(quad[next][1])
                    - f64::from(quad[next][0]) * f64::from(quad[i][1]);
            }
            coverage += (shoelace.abs() * 0.5) * weight;
        }
    }

    let average = lines.iter().map(|line| f64::from(line.score)).sum::<f64>() / lines.len() as f64;
    let quality = average * average * evidence.ln_1p() * (1.0 + 0.03 * coverage.ln_1p());
    (quality, average)
}

fn accept_region(lines: &mut Vec<PolicyLine>, region: Rect, candidate: Vec<PolicyLine>) {
    if candidate.is_empty() {
        return;
    }
    let previous: Vec<PolicyLine> = lines_in_region(lines, region)
        .into_iter()
        .cloned()
        .collect();
    if recognition_quality(&candidate) <= recognition_quality(&previous) {
        return;
    }
    let mut remaining: Vec<PolicyLine> = Vec::with_capacity(lines.len() + candidate.len());
    for line in lines.drain(..) {
        let remove = if let Some(bounds) = line_bounds(&line) {
            let region_area = rect_area(region) as f64;
            let bounds_area = rect_area(bounds) as f64;
            let in_region =
                (intersection(bounds, region) as f64) >= bounds_area.min(region_area) * 0.5;
            in_region && (intersection(bounds, region) as f64) >= bounds_area * 0.95
        } else {
            false
        };
        if !remove {
            remaining.push(line);
        }
    }
    remaining.extend(candidate);
    *lines = merge_tile_lines(remaining);
}

fn quad_bounds_f64(line: &PolicyLine) -> (f64, f64, f64, f64) {
    let Some(quad) = line.box_pts else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for [x, y] in quad {
        min_x = min_x.min(f64::from(x));
        min_y = min_y.min(f64::from(y));
        max_x = max_x.max(f64::from(x));
        max_y = max_y.max(f64::from(y));
    }
    (min_x, min_y, max_x, max_y)
}

pub fn merge_tile_lines(mut lines: Vec<PolicyLine>) -> Vec<PolicyLine> {
    lines.sort_by(|a, b| {
        (!b.tile_edge)
            .cmp(&(!a.tile_edge))
            .then_with(|| b.score.total_cmp(&a.score))
    });

    let mut kept: Vec<PolicyLine> = Vec::new();
    for line in lines {
        let (x0, y0, x1, y1) = quad_bounds_f64(&line);
        let mut duplicate = false;
        let mut replace_indices: Vec<usize> = Vec::new();

        for (prev_idx, previous) in kept.iter().enumerate() {
            let (a0, b0, a1, b1) = quad_bounds_f64(previous);
            let inter = (x1.min(a1) - x0.max(a0)).max(0.0) * (y1.min(b1) - y0.max(b0)).max(0.0);
            let area_cur = (x1 - x0) * (y1 - y0);
            let area_prev = (a1 - a0) * (b1 - b0);
            let smaller = area_cur.min(area_prev);
            if smaller > 0.0 && inter / smaller > 0.65 {
                let text: String = line.text.split_whitespace().collect();
                let previous_text: String = previous.text.split_whitespace().collect();
                if !previous_text.is_empty()
                    && previous_text != text
                    && text.contains(&previous_text)
                    && line.score >= 0.8
                    && area_cur >= area_prev
                {
                    replace_indices.push(prev_idx);
                    continue;
                }
                duplicate = true;
                break;
            }
        }
        if !duplicate {
            if !replace_indices.is_empty() {
                for idx in replace_indices.into_iter().rev() {
                    kept.remove(idx);
                }
            }
            kept.push(line);
        }
    }

    kept.sort_by(|a, b| {
        let ba = quad_bounds_f64(a);
        let bb = quad_bounds_f64(b);
        ba.1.total_cmp(&bb.1).then_with(|| ba.0.total_cmp(&bb.0))
    });

    let mut rows: Vec<Vec<PolicyLine>> = Vec::new();
    for line in kept {
        let (_, top, _, bottom) = quad_bounds_f64(&line);
        let same_row = if let Some(last_row) = rows.last() {
            let (_, row_top, _, row_bottom) = quad_bounds_f64(&last_row[0]);
            ((top + bottom) - (row_top + row_bottom)).abs()
                <= (bottom - top).min(row_bottom - row_top)
        } else {
            false
        };
        if same_row {
            rows.last_mut().expect("row").push(line);
        } else {
            rows.push(vec![line]);
        }
    }

    let mut merged: Vec<PolicyLine> = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| quad_bounds_f64(a).0.total_cmp(&quad_bounds_f64(b).0));
        let mut joined: Vec<PolicyLine> = Vec::new();
        for line in row {
            if let Some(previous) = joined.last_mut() {
                if previous.clip_right && line.clip_left {
                    let (a0, b0, a1, b1) = quad_bounds_f64(previous);
                    let (x0, y0, x1, y1) = quad_bounds_f64(&line);
                    let overlap = (a1.min(x1) - a0.max(x0)).max(0.0);
                    let left_chars: Vec<char> = previous.text.trim_end().chars().collect();
                    let right_chars: Vec<char> = line.text.trim_start().chars().collect();
                    let max_n = left_chars.len().min(right_chars.len());
                    let mut common = 0usize;
                    for n in (2..=max_n).rev() {
                        if left_chars[left_chars.len() - n..] == right_chars[..n] {
                            common = n;
                            break;
                        }
                    }
                    if overlap > 0.0 && common > 0 {
                        let stitched: String = left_chars
                            .into_iter()
                            .chain(right_chars.into_iter().skip(common))
                            .collect();
                        let top = b0.min(y0) as f32;
                        let bot = b1.max(y1) as f32;
                        let left = a0 as f32;
                        let right = a1.max(x1) as f32;
                        previous.text = stitched;
                        previous.score = previous.score.min(line.score);
                        previous.box_pts =
                            Some([[left, top], [right, top], [right, bot], [left, bot]]);
                        previous.clip_right = line.clip_right;
                        continue;
                    }
                }
            }
            joined.push(line);
        }
        merged.extend(joined);
    }

    for (index, line) in merged.iter_mut().enumerate() {
        line.index = index;
        line.tile_edge = false;
    }
    merged
}

fn save_visualization(image: &RgbImage, lines: &[PolicyLine], output: &Path) -> Result<(), String> {
    let mut canvas = image.clone();
    let w = canvas.width() as i32;
    let h = canvas.height() as i32;
    let color = Rgb([0, 200, 70]);
    for line in lines {
        let Some(quad) = line.box_pts else {
            continue;
        };
        for i in 0..4 {
            let [x0, y0] = quad[i];
            let [x1, y1] = quad[(i + 1) % 4];
            draw_line(
                &mut canvas,
                w,
                h,
                x0.round() as i32,
                y0.round() as i32,
                x1.round() as i32,
                y1.round() as i32,
                color,
            );
        }
    }
    if let Some(parent) = output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    DynamicImage::ImageRgb8(canvas).save(output).map_err(|err| {
        format!(
            "failed to write OCR visualization {}: {err}",
            output.display()
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    img: &mut RgbImage,
    w: i32,
    h: i32,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: Rgb<u8>,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        for oy in 0..=1 {
            for ox in 0..=1 {
                let px = x0 + ox;
                let py = y0 + oy;
                if px >= 0 && px < w && py >= 0 && py < h {
                    img.put_pixel(px as u32, py as u32, color);
                }
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn round6_f32(value: f32) -> f32 {
    (f64::from(value) * 1_000_000.0).round() as f32 / 1_000_000.0
}

// ---------------------------------------------------------------------------
// Headless `--ocr-worker` CLI for packaging validation, smoke tests, benchmarks
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WorkerStdioRequest {
    command: Option<String>,
    image: Option<String>,
    output: Option<String>,
    image_bytes_base64: Option<String>,
    lang: Option<String>,
    min_score: Option<f32>,
    vis: Option<String>,
    profile: Option<String>,
    max_side_len: Option<u32>,
    limit_side_len: Option<u32>,
}

pub fn run_worker_cli(args: &[String]) -> i32 {
    let mut stdio = false;
    let mut validate_installed = false;
    let mut rapidocr_root: Option<PathBuf> = None;
    let mut model_root: Option<PathBuf> = None;
    let mut image_path: Option<PathBuf> = None;
    let mut lang = "mixed".to_string();
    let mut profile = "balanced".to_string();
    let mut max_side_len: Option<u32> = None;
    let mut intra_op_threads = env_usize("KAIXIN_OCR_INTRA_OP_THREADS", default_intra_op_threads());
    let mut rec_batch_num = env_usize("KAIXIN_OCR_REC_BATCH_NUM", 8);
    let mut min_score = 0.5f32;
    let mut cache_engines = 2usize;
    let mut fast_mode = env_bool("KAIXIN_OCR_FAST_MODE", false);
    let mut vis: Option<PathBuf> = None;
    let mut pretty = false;

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--stdio" => stdio = true,
            "--validate-installed" => validate_installed = true,
            "--pretty" => pretty = true,
            "--fast-mode" => fast_mode = true,
            "--no-fast-mode" => fast_mode = false,
            "--cpu-mem-arena" | "--no-cpu-mem-arena" => {}
            "--rapidocr-root" if i + 1 < args.len() => {
                i += 1;
                rapidocr_root = Some(PathBuf::from(&args[i]));
            }
            "--model-root" if i + 1 < args.len() => {
                i += 1;
                model_root = Some(PathBuf::from(&args[i]));
            }
            "--lang" if i + 1 < args.len() => {
                i += 1;
                lang = args[i].clone();
            }
            "--profile" if i + 1 < args.len() => {
                i += 1;
                profile = args[i].clone();
            }
            "--max-side-len" if i + 1 < args.len() => {
                i += 1;
                max_side_len = args[i].parse().ok();
            }
            "--intra-op-threads" if i + 1 < args.len() => {
                i += 1;
                if let Ok(v) = args[i].parse::<usize>() {
                    intra_op_threads = v.max(1);
                }
            }
            "--rec-batch-num" if i + 1 < args.len() => {
                i += 1;
                if let Ok(v) = args[i].parse::<usize>() {
                    rec_batch_num = v.max(1);
                }
            }
            "--min-score" if i + 1 < args.len() => {
                i += 1;
                if let Ok(v) = args[i].parse::<f32>() {
                    min_score = v;
                }
            }
            "--cache-engines" if i + 1 < args.len() => {
                i += 1;
                if let Ok(v) = args[i].parse::<usize>() {
                    cache_engines = v.clamp(1, 2);
                }
            }
            "--vis" if i + 1 < args.len() => {
                i += 1;
                vis = Some(PathBuf::from(&args[i]));
            }
            "--provider" | "--inter-op-threads" | "--download-retries" if i + 1 < args.len() => {
                i += 1;
            }
            other if !other.starts_with('-') && image_path.is_none() => {
                image_path = Some(PathBuf::from(other));
            }
            _ => {}
        }
        i += 1;
    }

    if validate_installed && (rapidocr_root.is_some() || model_root.is_some()) {
        return emit_worker_error(
            "installed OCR validation requires the default runtime paths",
            pretty,
        );
    }
    let Some(root) = rapidocr_root.or_else(rapidocr_paths::rapidocr_root) else {
        return emit_worker_error(&rapidocr_paths::missing_root_message(), pretty);
    };
    if validate_installed {
        if let Err(err) = rapidocr_paths::validate_rapidocr_models_cached(&root) {
            return emit_worker_error(&err, pretty);
        }
    }
    let resolved_models = model_root.unwrap_or_else(|| rapidocr_paths::rapidocr_models_dir(&root));
    let config = NativeOcrConfig {
        rapidocr_root: root,
        model_root: resolved_models,
        intra_op_threads,
        rec_batch_num,
        cache_limit: cache_engines,
        fast_mode,
    };

    let mut service = match NativeOcrService::new(config) {
        Ok(svc) => svc,
        Err(err) => return emit_worker_error(&err, pretty),
    };

    if stdio {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        for line_res in stdin.lock().lines() {
            let Ok(raw_line) = line_res else {
                break;
            };
            let trimmed = raw_line.trim_start_matches('\u{feff}').trim();
            if trimmed.is_empty() {
                continue;
            }
            let response_json = match serde_json::from_str::<WorkerStdioRequest>(trimmed) {
                Err(err) => worker_error_json(&format!("invalid request JSON: {err}")),
                Ok(req) => match req.command.as_deref() {
                    Some("quit") => {
                        service.close();
                        return 0;
                    }
                    Some("warmup") => {
                        let req_profile =
                            PolicyProfile::parse(req.profile.as_deref().unwrap_or(&profile));
                        match service.warmup(req_profile) {
                            Ok(resp) => serde_json::to_string(&resp)
                                .unwrap_or_else(|e| worker_error_json(&e.to_string())),
                            Err(err) => worker_error_json(&err),
                        }
                    }
                    Some("crop") => run_worker_crop(req.image.as_deref(), req.output.as_deref()),
                    _ => {
                        let req_lang = req.lang.as_deref().unwrap_or(&lang);
                        let req_min_score = req.min_score.unwrap_or(min_score);
                        let req_profile =
                            PolicyProfile::parse(req.profile.as_deref().unwrap_or(&profile));
                        let req_max_side = req.max_side_len.or(req.limit_side_len).or(max_side_len);
                        let req_vis = req.vis.as_deref().map(PathBuf::from);
                        let result = if let Some(b64) = req.image_bytes_base64.as_deref() {
                            decode_base64_image(b64).and_then(|bytes| {
                                service.recognize(
                                    ImageInput::Bytes {
                                        bytes: &bytes,
                                        label: req.image.clone().unwrap_or_default(),
                                    },
                                    req_lang,
                                    req_min_score,
                                    req_profile,
                                    req_max_side,
                                    req_vis.as_deref(),
                                )
                            })
                        } else if let Some(path_str) = req.image.as_deref() {
                            service.recognize(
                                ImageInput::Path(Path::new(path_str)),
                                req_lang,
                                req_min_score,
                                req_profile,
                                req_max_side,
                                req_vis.as_deref(),
                            )
                        } else {
                            Err("image path or image_bytes_base64 is required".to_string())
                        };
                        match result {
                            Ok(resp) => serde_json::to_string(&resp)
                                .unwrap_or_else(|e| worker_error_json(&e.to_string())),
                            Err(err) => worker_error_json(&err),
                        }
                    }
                },
            };
            let _ = writeln!(stdout, "{response_json}");
            let _ = stdout.flush();
        }
        service.close();
        return 0;
    }

    let Some(img_path) = image_path else {
        return emit_worker_error("image path is required", pretty);
    };
    match service.recognize(
        ImageInput::Path(&img_path),
        &lang,
        min_score,
        PolicyProfile::parse(&profile),
        max_side_len,
        vis.as_deref(),
    ) {
        Ok(resp) => {
            let serialized = if pretty {
                serde_json::to_string_pretty(&resp)
            } else {
                serde_json::to_string(&resp)
            };
            match serialized {
                Ok(text) => {
                    println!("{text}");
                    0
                }
                Err(err) => emit_worker_error(&err.to_string(), pretty),
            }
        }
        Err(err) => emit_worker_error(&err, pretty),
    }
}

fn decode_base64_image(encoded: &str) -> Result<Vec<u8>, String> {
    if encoded.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err(format!(
            "encoded image is too large: {} bytes (limit {MAX_ENCODED_IMAGE_BYTES})",
            encoded.len()
        ));
    }
    let decoded = BASE64
        .decode(encoded)
        .map_err(|err| format!("invalid base64 image payload: {err}"))?;
    if decoded.len() > MAX_DECODED_IMAGE_BYTES {
        return Err(format!(
            "decoded image is too large: {} bytes (limit {MAX_DECODED_IMAGE_BYTES})",
            decoded.len()
        ));
    }
    Ok(decoded)
}

fn run_worker_crop(image_value: Option<&str>, output_value: Option<&str>) -> String {
    let Some(image_str) = image_value else {
        return worker_error_json("crop request missing image path");
    };
    let Some(output_str) = output_value else {
        return worker_error_json("crop request missing output path");
    };
    let source = Path::new(image_str);
    let output = Path::new(output_str);
    let Ok(decoded) = image::open(source) else {
        return worker_error_json(&format!(
            "image not found or unreadable: {}",
            source.display()
        ));
    };
    let rgb = decoded.to_rgb8();
    let width = rgb.width();
    let height = rgb.height();
    let Some(detected) = detect_window_border(&rgb, 6, 0.04, 0.985) else {
        return serde_json::json!({
            "ok": true,
            "command": "crop",
            "cropped": false,
            "reason": "no-confident-border",
            "width": width,
            "height": height,
        })
        .to_string();
    };
    let cropped = crop_rgb(
        &rgb,
        detected.x,
        detected.y,
        detected.width,
        detected.height,
    );
    if let Some(parent) = output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = DynamicImage::ImageRgb8(cropped).save(output) {
        return worker_error_json(&format!("failed to save cropped image: {err}"));
    }
    serde_json::json!({
        "ok": true,
        "command": "crop",
        "cropped": true,
        "output": output.display().to_string(),
        "rect": [detected.x, detected.y, detected.width, detected.height],
        "width": width,
        "height": height,
        "confidence": round6_f32(detected.confidence),
        "border_density": round6_f32(detected.border_density),
    })
    .to_string()
}

fn worker_error_json(message: &str) -> String {
    serde_json::json!({
        "ok": false,
        "engine": "rapidocr",
        "error": message,
    })
    .to_string()
}

fn emit_worker_error(message: &str, pretty: bool) -> i32 {
    let value = serde_json::json!({
        "ok": false,
        "engine": "rapidocr",
        "error": message,
    });
    let text = if pretty {
        serde_json::to_string_pretty(&value)
    } else {
        serde_json::to_string(&value)
    }
    .unwrap_or_else(|_| worker_error_json(message));
    println!("{text}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_line(
        text: &str,
        score: f32,
        [x0, y0, x1, y1]: [f32; 4],
        tile_edge: bool,
        clip_left: bool,
        clip_right: bool,
    ) -> PolicyLine {
        PolicyLine {
            index: 0,
            text: text.to_string(),
            score,
            box_pts: Some([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
            tile_edge,
            clip_left,
            clip_right,
        }
    }

    #[test]
    fn dedup_keeps_repeated_text_at_distinct_positions() {
        let lines = vec![
            mk_line("测试", 0.8, [0.0, 0.0, 40.0, 20.0], false, false, false),
            mk_line("测试", 0.95, [1.0, 0.0, 41.0, 20.0], false, false, false),
            mk_line("测试", 0.9, [100.0, 0.0, 140.0, 20.0], false, false, false),
        ];
        assert_eq!(merge_tile_lines(lines).len(), 2);
    }

    #[test]
    fn seam_prefix_cannot_replace_complete_line() {
        let clipped = mk_line(
            "跨越分块接缝",
            0.9999,
            [100.0, 0.0, 200.0, 20.0],
            true,
            false,
            false,
        );
        let full = mk_line(
            "跨越分块接缝的完整文字",
            0.99,
            [100.0, 0.0, 400.0, 20.0],
            true,
            false,
            false,
        );
        let merged = merge_tile_lines(vec![clipped, full]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "跨越分块接缝的完整文字");
    }

    #[test]
    fn seam_stitches_overlapping_suffix_and_prefix() {
        let left = mk_line(
            "第一段接缝文字",
            0.99,
            [0.0, 0.0, 150.0, 20.0],
            false,
            false,
            true,
        );
        let right = mk_line(
            "接缝文字继续识别",
            0.98,
            [110.0, 0.0, 260.0, 20.0],
            false,
            true,
            false,
        );
        let merged = merge_tile_lines(vec![left, right]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "第一段接缝文字继续识别");
    }

    #[test]
    fn vertical_padding_pads_short_line_and_maps_boxes_back() {
        let img = RgbImage::from_pixel(320, 20, Rgb([255, 255, 255]));
        let (padded, pad_top) = apply_vertical_padding(img, WIDTH_HEIGHT_RATIO, MIN_HEIGHT_PAD);
        assert!(pad_top > 0);
        assert_eq!(padded.height(), 20 + (pad_top as u32) * 2);
        let quad = map_quad_to_original(
            [
                [10.0, pad_top as f32 + 2.0],
                [100.0, pad_top as f32 + 2.0],
                [100.0, pad_top as f32 + 18.0],
                [10.0, pad_top as f32 + 18.0],
            ],
            1.0,
            1.0,
            pad_top,
            320.0,
            20.0,
        );
        assert!((quad[0][1] - 2.0).abs() < 0.01);
        assert!((quad[2][1] - 18.0).abs() < 0.01);
    }

    #[test]
    fn clahe_preserves_dimensions_and_increases_low_contrast_spread() {
        let mut img = RgbImage::from_pixel(64, 64, Rgb([120, 120, 120]));
        for y in 20..44 {
            for x in 10..54 {
                img.put_pixel(x, y, Rgb([135, 135, 135]));
            }
        }
        let enhanced = enhance_contrast_clahe(&img);
        assert_eq!(enhanced.dimensions(), (64, 64));
        let mn = enhanced.as_raw().iter().copied().min().unwrap();
        let mx = enhanced.as_raw().iter().copied().max().unwrap();
        assert!(mx - mn > 15);
    }
}
