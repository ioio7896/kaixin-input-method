use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, FindWindowW, ICONINFO, SM_CXSMICON,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Chinese,
    English,
    Gamepad,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IconKey {
    pub mode: Mode,
    pub failed: bool,
    pub size: i32,
}

pub(super) fn taskbar_icon_size() -> i32 {
    let dpi = unsafe {
        let class: Vec<u16> = "Shell_TrayWnd".encode_utf16().chain(Some(0)).collect();
        let taskbar = FindWindowW(class.as_ptr(), null());
        let dpi = GetDpiForWindow(taskbar);
        if dpi == 0 {
            GetDpiForSystem()
        } else {
            dpi
        }
    };
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi.max(96)) };
    if size == 0 {
        16
    } else {
        size.clamp(16, 64)
    }
}

pub(super) unsafe fn create(key: IconKey) -> Option<isize> {
    let size = key.size;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        },
        ..zeroed()
    };
    let mut bits = null_mut();
    let color = CreateDIBSection(0, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
    if color == 0 {
        return None;
    }
    if bits.is_null() {
        DeleteObject(color);
        return None;
    }
    let pixels = std::slice::from_raw_parts_mut(bits.cast::<[u8; 4]>(), (size * size) as usize);
    for (index, pixel) in pixels.iter_mut().enumerate() {
        *pixel = pixel_at(key, index as i32 % size, index as i32 / size);
    }
    // The 32-bit DIB provides transparency. The monochrome mask has word-aligned rows.
    let mask_stride = (size as usize).div_ceil(16);
    let mask_bits = vec![0u16; mask_stride * size as usize];
    let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
    let icon = if mask != 0 {
        let handle = CreateIconIndirect(&ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        });
        DeleteObject(mask);
        handle
    } else {
        0
    };
    DeleteObject(color);
    (icon != 0).then_some(icon)
}

fn pixel_at(key: IconKey, x: i32, y: i32) -> [u8; 4] {
    const SAMPLES: i32 = 4;
    let mut channels = [0u32; 4];
    let scale = 16.0 / key.size as f32;
    for sy in 0..SAMPLES {
        for sx in 0..SAMPLES {
            let x = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
            let y = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
            for (sum, value) in channels.iter_mut().zip(sample(key, x, y)) {
                *sum += u32::from(value);
            }
        }
    }
    // Averaging transparent black with opaque colors yields premultiplied BGRA.
    channels.map(|sum| ((sum + 8) / 16) as u8)
}

fn sample(key: IconKey, x: f32, y: f32) -> [u8; 4] {
    let point = (x, y);
    let rect = |l, t, r, b| (l..r).contains(&x) && (t..b).contains(&y);
    let circle =
        |cx: f32, cy: f32, radius: f32| (x - cx).powi(2) + (y - cy).powi(2) <= radius.powi(2);
    let ink = [255, 255, 255, 255];
    if key.failed && circle(12.5, 3.5, 2.55) {
        return if circle(12.5, 3.5, 1.75) {
            [63, 53, 215, 255]
        } else {
            ink
        };
    }
    let corner_x = ((x - 8.0).abs() - 4.5).max(0.0);
    let corner_y = ((y - 8.0).abs() - 4.5).max(0.0);
    if corner_x.powi(2) + corner_y.powi(2) >= 2.5_f32.powi(2) {
        return [0; 4];
    }
    let (background, glyph) = match key.mode {
        Mode::Chinese => (
            [136, 111, 15, 255],
            (rect(4.0, 5.0, 12.0, 10.5) && !rect(5.3, 6.3, 10.7, 9.2))
                || rect(7.25, 3.5, 8.75, 12.5),
        ),
        Mode::English => (
            [65, 53, 42, 255],
            stroke(point, (4.9, 11.5), (8.0, 4.4))
                || stroke(point, (8.0, 4.4), (11.1, 11.5))
                || rect(6.0, 8.6, 10.0, 10.0),
        ),
        Mode::Gamepad => {
            let body = rect(5.7, 5.1, 10.3, 9.9)
                || stroke(point, (5.7, 7.3), (4.6, 10.1))
                || stroke(point, (10.3, 7.3), (11.4, 10.1))
                || circle(5.5, 7.5, 2.25)
                || circle(10.5, 7.5, 2.25)
                || circle(4.9, 9.9, 1.8)
                || circle(11.1, 9.9, 1.8);
            let controls = rect(4.2, 7.2, 7.0, 8.2)
                || rect(5.1, 6.3, 6.1, 9.1)
                || circle(10.4, 7.1, 0.65)
                || circle(11.5, 8.6, 0.65);
            ([165, 61, 105, 255], body && !controls)
        }
        Mode::Unknown => ([107, 97, 87, 255], rect(4.9, 7.25, 11.1, 8.75)),
    };
    if glyph {
        ink
    } else {
        background
    }
}

fn stroke((x, y): (f32, f32), (x1, y1): (f32, f32), (x2, y2): (f32, f32)) -> bool {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let t = (((x - x1) * dx + (y - y1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (x - (x1 + t * dx)).powi(2) + (y - (y1 + t * dy)).powi(2) <= 0.85_f32.powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon;

    #[test]
    fn icon_sizes_have_transparent_corners_and_a_failure_only_badge() {
        for size in [16, 20, 24, 32] {
            for mode in [Mode::Chinese, Mode::English, Mode::Gamepad, Mode::Unknown] {
                let mut key = IconKey {
                    mode,
                    failed: false,
                    size,
                };
                assert_eq!(pixel_at(key, 0, 0), [0; 4]);
                let (x, y) = (size * 12 / 16, size * 3 / 16);
                let normal = pixel_at(key, x, y);
                key.failed = true;
                assert_eq!(pixel_at(key, x, y), [63, 53, 215, 255]);
                assert_ne!(normal, pixel_at(key, x, y));
                unsafe {
                    let icon = create(key).expect("valid native icon");
                    assert_ne!(DestroyIcon(icon), 0);
                }
            }
        }
    }
}
