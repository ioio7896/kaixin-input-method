//! Pure-Rust automatic outer window border detector.
//!
//! Direct port of the former `tools/kaixin_cv_crop.py` without OpenCV.

use crate::ocr_policy::{connected_components_8, rgb_to_gray, ComponentStat};
use image::RgbImage;

const MAX_OUTER_BORDER_INSET_RATIO: f64 = 0.04;
const MIN_OUTER_BORDER_INSET_PX: i32 = 12;

#[derive(Clone, Copy, Debug)]
pub struct BorderDetection {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub image_width: u32,
    pub image_height: u32,
    pub confidence: f32,
    pub border_density: f32,
}

pub fn detect_window_border(
    image: &RgbImage,
    padding: i32,
    min_area: f64,
    max_area: f64,
) -> Option<BorderDetection> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    if width < 48 || height < 48 {
        return None;
    }

    let gray = rgb_to_gray(image);
    let blurred = gaussian_blur_3x3(&gray, width, height);
    let edges = canny_edges(&blurred, width, height, 48.0, 144.0);
    let dilated = dilate_3x3(&edges, width, height);
    let closed = morph_close_3x3(&dilated, width, height, 2);
    let components = connected_components_8(&closed, width, height);

    let image_area = (width * height) as f64;
    let mut best: Option<(f64, ComponentStat, f64)> = None;

    for cc in components {
        if cc.w < 48 || cc.h < 48 {
            continue;
        }
        let rect_area = (cc.w * cc.h) as f64;
        let area_ratio = rect_area / image_area;
        if area_ratio < min_area || area_ratio > max_area {
            continue;
        }
        if cc.x <= 2 && cc.y <= 2 && cc.x + cc.w >= width - 2 && cc.y + cc.h >= height - 2 {
            continue;
        }

        let fill_ratio = (cc.area as f64 / rect_area.max(1.0)).clamp(0.0, 1.0);
        let density = border_density(&dilated, width, cc.x, cc.y, cc.w, cc.h);
        if density < 0.025 && fill_ratio < 0.35 {
            continue;
        }

        let corner_bonus = if corners_supported(&dilated, width, cc.x, cc.y, cc.w, cc.h) {
            1.15
        } else {
            1.0
        };
        let min_margin =
            cc.x.min(cc.y)
                .min(width.saturating_sub(cc.x + cc.w))
                .min(height.saturating_sub(cc.y + cc.h));
        let margin_penalty = if min_margin <= 2 { 0.9 } else { 1.0 };

        // Closed rectangular frames have high border density; reward strong
        // rectangular support similarly to `cv2.contourArea` on filled outer contours.
        let effective_fill = if density >= 0.45 {
            fill_ratio.max(0.85)
        } else {
            fill_ratio
        };
        let score = area_ratio
            * (0.55 + density * 3.0)
            * (0.55 + effective_fill)
            * corner_bonus
            * margin_penalty;

        if best.is_none_or(|(best_score, _, _)| score > best_score) {
            best = Some((score, cc, density));
        }
    }

    let (score, rect, density) = best?;
    let [x0, y0, x1, y1] = safe_border_crop_rect(
        rect.x as i32,
        rect.y as i32,
        rect.w as i32,
        rect.h as i32,
        width as i32,
        height as i32,
        padding,
    )?;

    Some(BorderDetection {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
        image_width: width as u32,
        image_height: height as u32,
        confidence: score as f32,
        border_density: density as f32,
    })
}

pub fn safe_border_crop_rect(
    x: i32,
    y: i32,
    rect_width: i32,
    rect_height: i32,
    image_width: i32,
    image_height: i32,
    padding: i32,
) -> Option<[i32; 4]> {
    let padding = padding.max(0);
    let x0 = (x - padding).max(0);
    let y0 = (y - padding).max(0);
    let x1 = (x + rect_width + padding).min(image_width);
    let y1 = (y + rect_height + padding).min(image_height);
    if (x1 - x0) >= image_width - 3 && (y1 - y0) >= image_height - 3 {
        return None;
    }
    let max_inset_x = MIN_OUTER_BORDER_INSET_PX
        .max(((image_width as f64) * MAX_OUTER_BORDER_INSET_RATIO).round() as i32);
    let max_inset_y = MIN_OUTER_BORDER_INSET_PX
        .max(((image_height as f64) * MAX_OUTER_BORDER_INSET_RATIO).round() as i32);
    if x0 > max_inset_x || (image_width - x1) > max_inset_x {
        return None;
    }
    if y0 > max_inset_y || (image_height - y1) > max_inset_y {
        return None;
    }
    Some([x0, y0, x1, y1])
}

fn border_density(edges: &[u8], stride: usize, x: usize, y: usize, w: usize, h: usize) -> f64 {
    if w == 0 || h == 0 {
        return 0.0;
    }
    let strip = (w.min(h) / 80).clamp(2, 6).min(w).min(h);
    let mut pixels = 0usize;
    let mut hits = 0usize;

    // Top & bottom strips
    for row in (y..y + strip).chain((y + h - strip)..(y + h)) {
        let off = row * stride;
        for col in x..(x + w) {
            pixels += 1;
            if edges[off + col] != 0 {
                hits += 1;
            }
        }
    }
    // Left & right strips
    for row in y..(y + h) {
        let off = row * stride;
        for col in (x..x + strip).chain((x + w - strip)..(x + w)) {
            pixels += 1;
            if edges[off + col] != 0 {
                hits += 1;
            }
        }
    }

    if pixels == 0 {
        0.0
    } else {
        hits as f64 / pixels as f64
    }
}

fn corners_supported(edges: &[u8], stride: usize, x: usize, y: usize, w: usize, h: usize) -> bool {
    let pad = (w.min(h) / 40).clamp(4, 10);
    let corners = [
        (x, y),
        (x + w.saturating_sub(pad), y),
        (x, y + h.saturating_sub(pad)),
        (x + w.saturating_sub(pad), y + h.saturating_sub(pad)),
    ];
    corners.into_iter().all(|(cx, cy)| {
        for row in cy..(cy + pad).min(y + h) {
            let off = row * stride;
            for col in cx..(cx + pad).min(x + w) {
                if edges[off + col] != 0 {
                    return true;
                }
            }
        }
        false
    })
}

fn gaussian_blur_3x3(gray: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut temp = vec![0u16; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let xl = x.saturating_sub(1);
            let xr = (x + 1).min(w - 1);
            temp[row + x] = u16::from(gray[row + xl])
                + 2 * u16::from(gray[row + x])
                + u16::from(gray[row + xr]);
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let yt = y.saturating_sub(1) * w;
        let ym = y * w;
        let yb = (y + 1).min(h - 1) * w;
        for x in 0..w {
            let sum = temp[yt + x] + 2 * temp[ym + x] + temp[yb + x];
            out[ym + x] = ((sum + 8) >> 4) as u8;
        }
    }
    out
}

fn canny_edges(gray: &[u8], w: usize, h: usize, low: f32, high: f32) -> Vec<u8> {
    let mut mag = vec![0.0f32; w * h];
    let mut dir = vec![0u8; w * h];

    for y in 1..h.saturating_sub(1) {
        let r0 = (y - 1) * w;
        let r1 = y * w;
        let r2 = (y + 1) * w;
        for x in 1..w.saturating_sub(1) {
            let tl = f32::from(gray[r0 + x - 1]);
            let tc = f32::from(gray[r0 + x]);
            let tr = f32::from(gray[r0 + x + 1]);
            let ml = f32::from(gray[r1 + x - 1]);
            let mr = f32::from(gray[r1 + x + 1]);
            let bl = f32::from(gray[r2 + x - 1]);
            let bc = f32::from(gray[r2 + x]);
            let br = f32::from(gray[r2 + x + 1]);

            let gx = -tl + tr - 2.0 * ml + 2.0 * mr - bl + br;
            let gy = -tl - 2.0 * tc - tr + bl + 2.0 * bc + br;
            // OpenCV default Canny uses L1 norm |gx| + |gy|.
            mag[r1 + x] = gx.abs() + gy.abs();

            let angle = gy.atan2(gx).to_degrees();
            let a = if angle < 0.0 { angle + 180.0 } else { angle };
            dir[r1 + x] = if !(22.5..157.5).contains(&a) {
                0
            } else if a < 67.5 {
                1
            } else if a < 112.5 {
                2
            } else {
                3
            };
        }
    }

    let mut edges = vec![0u8; w * h];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for y in 1..h.saturating_sub(1) {
        let row = y * w;
        for x in 1..w.saturating_sub(1) {
            let idx = row + x;
            let m = mag[idx];
            if m < low {
                continue;
            }
            let (n1, n2) = match dir[idx] {
                0 => (mag[idx - 1], mag[idx + 1]),
                1 => (mag[(y - 1) * w + x - 1], mag[(y + 1) * w + x + 1]),
                2 => (mag[(y - 1) * w + x], mag[(y + 1) * w + x]),
                _ => (mag[(y - 1) * w + x + 1], mag[(y + 1) * w + x - 1]),
            };
            if m >= n1 && m >= n2 {
                if m >= high {
                    edges[idx] = 255;
                    stack.push((x, y));
                } else {
                    edges[idx] = 1; // weak candidate
                }
            }
        }
    }

    while let Some((x, y)) = stack.pop() {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(h - 1);
        let x0 = x.saturating_sub(1);
        let x1 = (x + 1).min(w - 1);
        for ny in y0..=y1 {
            let nrow = ny * w;
            for nx in x0..=x1 {
                let idx = nrow + nx;
                if edges[idx] == 1 {
                    edges[idx] = 255;
                    stack.push((nx, ny));
                }
            }
        }
    }

    for px in &mut edges {
        if *px != 255 {
            *px = 0;
        }
    }
    edges
}

fn dilate_3x3(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(h - 1);
        for x in 0..w {
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);
            let mut hit = 0u8;
            'outer: for yy in y0..=y1 {
                let row = yy * w;
                for xx in x0..=x1 {
                    if src[row + xx] != 0 {
                        hit = 255;
                        break 'outer;
                    }
                }
            }
            out[y * w + x] = hit;
        }
    }
    out
}

fn erode_3x3(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(h - 1);
        for x in 0..w {
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);
            let mut all = 255u8;
            'outer: for yy in y0..=y1 {
                let row = yy * w;
                for xx in x0..=x1 {
                    if src[row + xx] == 0 {
                        all = 0;
                        break 'outer;
                    }
                }
            }
            out[y * w + x] = all;
        }
    }
    out
}

fn morph_close_3x3(src: &[u8], w: usize, h: usize, iterations: usize) -> Vec<u8> {
    let mut cur = src.to_vec();
    for _ in 0..iterations {
        cur = dilate_3x3(&cur, w, h);
    }
    for _ in 0..iterations {
        cur = erode_3x3(&cur, w, h);
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn detects_outer_window_border_and_rejects_inner_card() {
        // Outer border within 4% inset
        let mut img = RgbImage::from_pixel(400, 300, Rgb([245, 245, 245]));
        for y in 10..290 {
            for x in 10..390 {
                img.put_pixel(x, y, Rgb([30, 30, 30]));
            }
        }
        let detected = detect_window_border(&img, 2, 0.04, 0.985).expect("outer border");
        assert!(detected.x <= 12 && detected.y <= 12);
        assert!(detected.width >= 370 && detected.height >= 270);
        assert!(detected.confidence > 0.70);

        // Inner card far from edges must be rejected by safe_border_crop_rect
        let mut inner = RgbImage::from_pixel(400, 300, Rgb([245, 245, 245]));
        for y in 60..240 {
            for x in 80..320 {
                inner.put_pixel(x, y, Rgb([20, 20, 20]));
            }
        }
        assert!(detect_window_border(&inner, 6, 0.04, 0.985).is_none());
    }
}
