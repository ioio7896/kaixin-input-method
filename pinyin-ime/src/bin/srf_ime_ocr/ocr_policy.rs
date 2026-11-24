//! Bounded image planning and lightweight pure-Rust text-region analysis.
//!
//! Direct port of the former `tools/kaixin_ocr_policy.py` without OpenCV.

use image::RgbImage;
use serde::Serialize;
use std::time::Instant;

pub const MAX_TILES: usize = 32;
pub const MAX_WORK_PIXELS: u64 = 64_000_000;
pub const MAX_REFINEMENT_PIXELS: u64 = 8_000_000;
pub const MAX_REFINEMENTS: usize = 8;

/// `[x0, y0, x1, y1]` in source-image pixel coordinates.
pub type Rect = [i32; 4];

#[derive(Clone, Debug)]
pub struct PolicyLine {
    pub index: usize,
    pub text: String,
    pub score: f32,
    pub box_pts: Option<[[f32; 2]; 4]>,
    pub tile_edge: bool,
    pub clip_left: bool,
    pub clip_right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyProfile {
    Fast,
    Balanced,
    Accurate,
}

impl PolicyProfile {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "fast" => Self::Fast,
            "accurate" => Self::Accurate,
            _ => Self::Balanced,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Accurate => "accurate",
        }
    }

    pub fn limits(self) -> (u32, f32) {
        match self {
            Self::Fast => (1280, 1.0),
            Self::Balanced => (1920, 1.5),
            Self::Accurate => (2560, 2.0),
        }
    }

    pub fn models(self) -> (&'static str, &'static str) {
        match self {
            Self::Fast => ("small", "small"),
            Self::Balanced => ("small", "medium"),
            Self::Accurate => ("medium", "medium"),
        }
    }

    pub fn budget_seconds(self) -> f64 {
        match self {
            Self::Fast => 30.0,
            Self::Balanced => 60.0,
            Self::Accurate => 90.0,
        }
    }
}

pub fn rect_area(rect: Rect) -> u64 {
    let w = (rect[2] - rect[0]).max(0) as u64;
    let h = (rect[3] - rect[1]).max(0) as u64;
    w * h
}

pub fn image_tiles(width: u32, height: u32, limit: u32) -> Result<Vec<Rect>, String> {
    let width = width as i32;
    let height = height as i32;
    let limit = limit.clamp(320, 1920) as i32;
    let overlap = (limit / 10).clamp(128, 192);

    let segments = |length: i32| -> Vec<(i32, i32)> {
        if length <= limit {
            return vec![(0, length)];
        }
        let step = (limit - overlap).max(1) as f64;
        let count = ((length as f64) / step).ceil() as i32;
        (0..count)
            .map(|i| {
                let start = (((length as f64) * (i as f64) / (count as f64)).round() as i32
                    - overlap / 2)
                    .max(0);
                let end = (((length as f64) * ((i + 1) as f64) / (count as f64)).round() as i32
                    + overlap / 2)
                    .min(length);
                (start, end)
            })
            .collect()
    };

    let mut tiles = Vec::new();
    for (y0, y1) in segments(height) {
        for (x0, x1) in segments(width) {
            tiles.push([x0, y0, x1, y1]);
        }
    }
    let total_pixels: u64 = tiles.iter().copied().map(rect_area).sum();
    if tiles.len() > MAX_TILES || total_pixels > MAX_WORK_PIXELS {
        return Err("截图分块超出处理预算，请缩小选区或分次识别。".to_string());
    }
    Ok(tiles)
}

pub fn line_bounds(line: &PolicyLine) -> Option<Rect> {
    let quad = line.box_pts?;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for [x, y] in quad {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    Some([
        min_x.floor() as i32,
        min_y.floor() as i32,
        max_x.ceil() as i32,
        max_y.ceil() as i32,
    ])
}

pub fn intersection(a: Rect, b: Rect) -> u64 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0) as u64;
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0) as u64;
    w * h
}

#[derive(Clone, Debug, Default)]
pub struct ImageAnalysis {
    pub components: usize,
    pub small_components: usize,
    pub contrast: f64,
    pub regions: Vec<Rect>,
    pub small_regions: Vec<Rect>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImageAnalysisSummary {
    pub components: usize,
    pub small_components: usize,
    pub contrast: f64,
    pub low_contrast: bool,
    pub regions: usize,
    pub small_regions: usize,
}

impl ImageAnalysis {
    pub fn low_contrast(&self) -> bool {
        self.components >= 2 && self.contrast > 3.0 && self.contrast < 65.0
    }

    pub fn summary(&self) -> ImageAnalysisSummary {
        ImageAnalysisSummary {
            components: self.components,
            small_components: self.small_components,
            contrast: (self.contrast * 100.0).round() / 100.0,
            low_contrast: self.low_contrast(),
            regions: self.regions.len(),
            small_regions: self.small_regions.len(),
        }
    }
}

pub fn analyze_image(image: &RgbImage) -> ImageAnalysis {
    let width = image.width() as usize;
    let height = image.height() as usize;
    if width == 0 || height == 0 {
        return ImageAnalysis::default();
    }

    let scale = (2048.0_f64 / (width.max(height) as f64)).min(1.0);
    let raw_gray = rgb_to_gray(image);
    let (gw, gh, gray) = if scale < 1.0 {
        let dst_w = ((width as f64) * scale).round().max(1.0) as usize;
        let dst_h = ((height as f64) * scale).round().max(1.0) as usize;
        let resized = resize_gray_area(&raw_gray, width, height, dst_w, dst_h);
        (dst_w, dst_h, resized)
    } else {
        (width, height, raw_gray)
    };

    let gradient = morph_gradient_3x3(&gray, gw, gh);
    let max_grad = gradient.iter().copied().max().unwrap_or(0);
    if max_grad < 4 {
        return ImageAnalysis::default();
    }

    let thresh = otsu_threshold(&gradient);
    let mask: Vec<u8> = gradient
        .iter()
        .map(|&v| if v > thresh { 255 } else { 0 })
        .collect();
    let stats = connected_components_8(&mask, gw, gh);

    let mut glyphs = 0usize;
    let mut small = 0usize;
    let mut text_mask = vec![0u8; gw * gh];
    let mut small_mask = vec![0u8; gw * gh];
    let mut strengths = Vec::new();

    for cc in stats {
        let source_h = (((cc.h as i32) - 2) as f64 / scale).max(1.0);
        let box_area = (cc.w * cc.h) as f64;
        let density = (cc.area as f64) / box_area.max(1.0);
        if !(cc.h >= 3
            && (5.0..=64.0).contains(&source_h)
            && cc.w >= 2
            && cc.w <= cc.h * 5
            && cc.area >= 5
            && (0.12..=1.0).contains(&density))
        {
            continue;
        }
        glyphs += 1;
        let mut local_max = 0u8;
        for row in cc.y..(cc.y + cc.h) {
            let row_off = row * gw;
            for col in cc.x..(cc.x + cc.w) {
                let idx = row_off + col;
                text_mask[idx] = 255;
                if gradient[idx] > local_max {
                    local_max = gradient[idx];
                }
            }
        }
        strengths.push(local_max as f64);
        if source_h <= 19.0 {
            small += 1;
            for row in cc.y..(cc.y + cc.h) {
                let row_off = row * gw;
                for col in cc.x..(cc.x + cc.w) {
                    small_mask[row_off + col] = 255;
                }
            }
        }
    }

    let kw = ((22.0 * scale).round() as usize).max(9);
    let groups = |source: &[u8]| -> Vec<Rect> {
        let joined = dilate_rect_binary(source, gw, gh, kw, 3);
        let regions = connected_components_8(&joined, gw, gh);
        let mut output = Vec::new();
        for cc in regions {
            if (cc.w as f64) / scale < 24.0 || (cc.h as f64) / scale > 192.0 || cc.area < 18 {
                continue;
            }
            let x0 = (((cc.x as f64) / scale).floor() as i32 - 8).max(0);
            let y0 = (((cc.y as f64) / scale).floor() as i32 - 6).max(0);
            let x1 = ((((cc.x + cc.w) as f64) / scale).ceil() as i32 + 8).min(width as i32);
            let y1 = ((((cc.y + cc.h) as f64) / scale).ceil() as i32 + 6).min(height as i32);
            output.push([x0, y0, x1, y1]);
        }
        output.sort_by_key(|r| (r[1], r[0]));
        output
    };

    let contrast = median_f64(&mut strengths);
    let regions = groups(&text_mask);
    let small_regions = if small >= 3 {
        groups(&small_mask)
    } else {
        Vec::new()
    };

    ImageAnalysis {
        components: glyphs,
        small_components: small,
        contrast,
        regions,
        small_regions,
    }
}

pub fn lines_in_region(lines: &[PolicyLine], region: Rect) -> Vec<&PolicyLine> {
    let region_area = rect_area(region) as f64;
    lines
        .iter()
        .filter(|line| {
            let Some(bounds) = line_bounds(line) else {
                return false;
            };
            let min_area = (rect_area(bounds) as f64).min(region_area);
            (intersection(bounds, region) as f64) >= min_area * 0.5
        })
        .collect()
}

pub fn needs_detector_retry(lines: &[PolicyLine], analysis: &ImageAnalysis) -> bool {
    if analysis.components < 3 || analysis.regions.is_empty() {
        return false;
    }
    let missed = analysis
        .regions
        .iter()
        .filter(|&&region| lines_in_region(lines, region).is_empty())
        .count();
    let threshold = ((analysis.regions.len() as f64) * 0.4).ceil() as usize;
    missed >= threshold.max(1)
}

pub fn should_retry_preprocess(lines: &[PolicyLine], analysis: &ImageAnalysis) -> bool {
    if !analysis.low_contrast() {
        return false;
    }
    if lines.is_empty() {
        return true;
    }
    let avg: f64 = lines.iter().map(|line| f64::from(line.score)).sum::<f64>() / lines.len() as f64;
    avg < 0.82
}

pub fn refinement_regions(lines: &[PolicyLine], analysis: &ImageAnalysis) -> Vec<Rect> {
    let mut candidates: Vec<(f64, Rect)> = Vec::new();
    for &region in &analysis.small_regions {
        let covered = lines_in_region(lines, region);
        let confidence = if covered.is_empty() {
            0.0
        } else {
            covered
                .iter()
                .map(|line| f64::from(line.score))
                .sum::<f64>()
                / covered.len() as f64
        };
        if confidence < 0.94 {
            candidates.push((confidence, region));
        }
    }
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1[1].cmp(&b.1[1]))
            .then_with(|| a.1[0].cmp(&b.1[0]))
    });
    candidates
        .into_iter()
        .take(MAX_REFINEMENTS)
        .map(|(_, rect)| rect)
        .collect()
}

pub struct WorkBudget {
    started: Instant,
    limit_seconds: f64,
    pub pixels: u64,
    pub extra_pixels: u64,
    pub extra_calls: usize,
    pub skipped: Vec<String>,
}

impl WorkBudget {
    pub fn new(seconds: f64) -> Self {
        Self {
            started: Instant::now(),
            limit_seconds: seconds,
            pixels: 0,
            extra_pixels: 0,
            extra_calls: 0,
            skipped: Vec::new(),
        }
    }

    pub fn reserve(&mut self, pixels: u64, kind: &str, optional: bool) -> Result<bool, String> {
        self.reserve_at(self.started.elapsed().as_secs_f64(), pixels, kind, optional)
    }

    fn reserve_at(
        &mut self,
        elapsed_seconds: f64,
        pixels: u64,
        kind: &str,
        optional: bool,
    ) -> Result<bool, String> {
        let reason = if elapsed_seconds >= self.limit_seconds {
            Some("time")
        } else if self.pixels.saturating_add(pixels) > MAX_WORK_PIXELS {
            Some("pixels")
        } else if optional
            && (self.extra_calls >= MAX_REFINEMENTS
                || self.extra_pixels.saturating_add(pixels) > MAX_REFINEMENT_PIXELS)
        {
            Some("refinement")
        } else {
            None
        };

        if let Some(reason) = reason {
            if !optional {
                return Err("OCR 处理超出时间或像素预算，请缩小选区后重试。".to_string());
            }
            self.skipped.push(format!("{kind}:{reason}"));
            return Ok(false);
        }

        self.pixels = self.pixels.saturating_add(pixels);
        if optional {
            self.extra_pixels = self.extra_pixels.saturating_add(pixels);
            self.extra_calls += 1;
        }
        Ok(true)
    }
}

pub(crate) fn rgb_to_gray(image: &RgbImage) -> Vec<u8> {
    let raw = image.as_raw();
    let mut out = Vec::with_capacity((image.width() * image.height()) as usize);
    for px in raw.as_chunks::<3>().0 {
        let luma =
            (77u32 * u32::from(px[0]) + 150u32 * u32::from(px[1]) + 29u32 * u32::from(px[2]) + 128)
                >> 8;
        out.push(luma as u8);
    }
    out
}

fn resize_gray_area(src: &[u8], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let mut out = vec![0u8; dst_w * dst_h];
    let scale_x = src_w as f64 / dst_w as f64;
    let scale_y = src_h as f64 / dst_h as f64;
    for dy in 0..dst_h {
        let sy0 = ((dy as f64) * scale_y).floor() as usize;
        let sy1 = ((((dy + 1) as f64) * scale_y).ceil() as usize)
            .max(sy0 + 1)
            .min(src_h);
        for dx in 0..dst_w {
            let sx0 = ((dx as f64) * scale_x).floor() as usize;
            let sx1 = ((((dx + 1) as f64) * scale_x).ceil() as usize)
                .max(sx0 + 1)
                .min(src_w);
            let mut sum = 0u32;
            let mut count = 0u32;
            for sy in sy0..sy1 {
                let row = sy * src_w;
                for sx in sx0..sx1 {
                    sum += u32::from(src[row + sx]);
                    count += 1;
                }
            }
            out[dy * dst_w + dx] = ((sum + count / 2) / count.max(1)) as u8;
        }
    }
    out
}

fn morph_gradient_3x3(gray: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(h - 1);
        for x in 0..w {
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);
            let mut mn = 255u8;
            let mut mx = 0u8;
            for yy in y0..=y1 {
                let row = yy * w;
                for xx in x0..=x1 {
                    let v = gray[row + xx];
                    if v < mn {
                        mn = v;
                    }
                    if v > mx {
                        mx = v;
                    }
                }
            }
            out[y * w + x] = mx - mn;
        }
    }
    out
}

fn otsu_threshold(gray: &[u8]) -> u8 {
    let mut hist = [0u64; 256];
    for &v in gray {
        hist[usize::from(v)] += 1;
    }
    let total = gray.len() as f64;
    if total == 0.0 {
        return 0;
    }
    let mut sum_all = 0.0f64;
    for (i, &c) in hist.iter().enumerate() {
        sum_all += (i as f64) * (c as f64);
    }

    let mut weight_b = 0.0f64;
    let mut sum_b = 0.0f64;
    let mut max_var = -1.0f64;
    let mut best = 0u8;

    for (t, &c) in hist.iter().enumerate() {
        weight_b += c as f64;
        if weight_b == 0.0 {
            continue;
        }
        let weight_f = total - weight_b;
        if weight_f == 0.0 {
            break;
        }
        sum_b += (t as f64) * (c as f64);
        let mean_b = sum_b / weight_b;
        let mean_f = (sum_all - sum_b) / weight_f;
        let diff = mean_b - mean_f;
        let between = weight_b * weight_f * diff * diff;
        if between > max_var {
            max_var = between;
            best = t as u8;
        }
    }
    best
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ComponentStat {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub area: usize,
}

pub(crate) fn connected_components_8(mask: &[u8], w: usize, h: usize) -> Vec<ComponentStat> {
    let mut visited = vec![0u8; w * h];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut stats = Vec::new();

    for sy in 0..h {
        let row_off = sy * w;
        for sx in 0..w {
            let start = row_off + sx;
            if mask[start] == 0 || visited[start] != 0 {
                continue;
            }
            visited[start] = 1;
            stack.push((sx, sy));
            let mut min_x = sx;
            let mut max_x = sx;
            let mut min_y = sy;
            let mut max_y = sy;
            let mut area = 0usize;

            while let Some((x, y)) = stack.pop() {
                area += 1;
                if x < min_x {
                    min_x = x;
                }
                if x > max_x {
                    max_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if y > max_y {
                    max_y = y;
                }

                let ny0 = y.saturating_sub(1);
                let ny1 = (y + 1).min(h - 1);
                let nx0 = x.saturating_sub(1);
                let nx1 = (x + 1).min(w - 1);
                for ny in ny0..=ny1 {
                    let nrow = ny * w;
                    for nx in nx0..=nx1 {
                        let idx = nrow + nx;
                        if mask[idx] != 0 && visited[idx] == 0 {
                            visited[idx] = 1;
                            stack.push((nx, ny));
                        }
                    }
                }
            }

            stats.push(ComponentStat {
                x: min_x,
                y: min_y,
                w: max_x - min_x + 1,
                h: max_y - min_y + 1,
                area,
            });
        }
    }
    stats
}

fn dilate_rect_binary(src: &[u8], w: usize, h: usize, kw: usize, kh: usize) -> Vec<u8> {
    let rx_left = kw / 2;
    let rx_right = kw.saturating_sub(1) - rx_left;
    let ry_top = kh / 2;
    let ry_bottom = kh.saturating_sub(1) - ry_top;

    // Separable O(N) binary dilation via prefix sums: horizontal pass then vertical pass.
    let mut horiz = vec![0u8; w * h];
    let mut prefix = vec![0u32; w + 1];
    for y in 0..h {
        let row = y * w;
        prefix[0] = 0;
        for x in 0..w {
            prefix[x + 1] = prefix[x] + u32::from(src[row + x] != 0);
        }
        for x in 0..w {
            let x0 = x.saturating_sub(rx_left);
            let x1 = (x + rx_right + 1).min(w);
            if prefix[x1] > prefix[x0] {
                horiz[row + x] = 255;
            }
        }
    }

    let mut out = vec![0u8; w * h];
    let mut col_prefix = vec![0u32; h + 1];
    for x in 0..w {
        col_prefix[0] = 0;
        for y in 0..h {
            col_prefix[y + 1] = col_prefix[y] + u32::from(horiz[y * w + x] != 0);
        }
        for y in 0..h {
            let y0 = y.saturating_sub(ry_top);
            let y1 = (y + ry_bottom + 1).min(h);
            if col_prefix[y1] > col_prefix[y0] {
                out[y * w + x] = 255;
            }
        }
    }
    out
}

fn median_f64(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_line(text: &str, score: f32, rect: [f32; 4]) -> PolicyLine {
        PolicyLine {
            index: 0,
            text: text.to_string(),
            score,
            box_pts: Some([
                [rect[0], rect[1]],
                [rect[2], rect[1]],
                [rect[2], rect[3]],
                [rect[0], rect[3]],
            ]),
            tile_edge: false,
            clip_left: false,
            clip_right: false,
        }
    }

    #[test]
    fn four_k_has_full_coverage_with_bounded_overlap() {
        let tiles = image_tiles(3840, 2160, 1920).unwrap();
        assert!(tiles.len() > 1);
        let total: u64 = tiles.iter().copied().map(rect_area).sum();
        assert!((total as f64) < (3840.0 * 2160.0 * 1.3));
        for x in [0, 1183, 1280, 1376, 1920, 2464, 2560, 2656, 3839] {
            for y in [0, 984, 1080, 1176, 2159] {
                assert!(tiles
                    .iter()
                    .any(|&[x0, y0, x1, y1]| x0 <= x && x < x1 && y0 <= y && y < y1));
            }
        }
        assert!(tiles
            .iter()
            .all(|&[x0, y0, x1, y1]| (x1 - x0).max(y1 - y0) <= 1920));
    }

    #[test]
    fn boundary_does_not_duplicate_whole_image() {
        let tiles = image_tiles(1921, 1921, 1920).unwrap();
        let total: u64 = tiles.iter().copied().map(rect_area).sum();
        assert!((total as f64) < (1921.0 * 1921.0 * 1.3));
        assert_eq!(image_tiles(360, 48, 1920).unwrap(), vec![[0, 0, 360, 48]]);
    }

    #[test]
    fn huge_plan_is_rejected_without_partial_output() {
        assert!(image_tiles(16384, 16384, 1280).is_err());
    }

    #[test]
    fn optional_budget_stops_but_primary_fails_closed() {
        let mut budget = WorkBudget::new(10.0);
        for _ in 0..MAX_REFINEMENTS {
            assert!(budget.reserve_at(0.0, 100, "small-text", true).unwrap());
        }
        assert!(!budget.reserve_at(0.0, 100, "small-text", true).unwrap());
        assert!(budget.reserve_at(0.0, 100, "primary", false).unwrap());
        assert!(!budget.reserve_at(11.0, 100, "contrast", true).unwrap());
        assert!(budget.reserve_at(11.0, 100, "primary", false).is_err());
        let mut fresh = WorkBudget::new(10.0);
        assert!(fresh
            .reserve_at(0.0, MAX_WORK_PIXELS + 1, "primary", false)
            .is_err());
    }

    #[test]
    fn valid_single_character_is_not_retry_trigger() {
        let good = vec![mk_line("好", 0.99, [0.0, 0.0, 20.0, 20.0])];
        assert!(!should_retry_preprocess(&good, &ImageAnalysis::default()));
        let low = ImageAnalysis {
            components: 5,
            small_components: 0,
            contrast: 40.0,
            regions: Vec::new(),
            small_regions: Vec::new(),
        };
        assert!(!should_retry_preprocess(&good, &low));
        let bad = vec![mk_line("?", 0.4, [0.0, 0.0, 20.0, 20.0])];
        assert!(should_retry_preprocess(&bad, &low));
        assert!(!should_retry_preprocess(&[], &ImageAnalysis::default()));
    }

    #[test]
    fn small_component_inside_recognized_line_is_not_duplicated() {
        let lines = vec![mk_line("金额 128.50 元", 0.99, [0.0, 0.0, 300.0, 30.0])];
        let analysis = ImageAnalysis {
            components: 8,
            small_components: 4,
            contrast: 255.0,
            regions: vec![[270, 0, 300, 30]],
            small_regions: vec![[270, 0, 300, 30]],
        };
        assert!(!needs_detector_retry(&lines, &analysis));
        assert!(refinement_regions(&lines, &analysis).is_empty());
    }
}
