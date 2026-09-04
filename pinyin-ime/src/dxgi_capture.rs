//! DXGI Desktop Duplication screenshot fallback.
//!
//! This backend captures display outputs and composes them in Windows virtual
//! desktop coordinates. A window capture is consequently a crop of the
//! composed desktop, so windows covering the requested HWND are visible in the
//! result. WGC remains the preferred path for isolated window capture.

use crate::windows_graphics_capture::{
    CaptureError, CaptureErrorKind, CaptureRect, CapturedFrame, CapturedSource, MonitorInfo,
};
use image::{Rgba, RgbaImage};
use std::time::{Duration, Instant};

// This is a fallback after WGC has already failed. Bound the per-display wait
// so a multi-monitor capture does not appear to hang for several seconds.
const CAPTURE_TIMEOUT_MS: u32 = 500;
const MAX_CAPTURE_PIXELS: u64 = 134_217_728;

#[cfg(windows)]
static CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(windows)]
use windows::{
    core::{Interface, Result as WindowsResult},
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_IDENTITY,
                    DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270,
                    DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_UNSPECIFIED,
                },
                CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1,
                IDXGIOutputDuplication, IDXGIResource, IDXGISurface1, DXGI_ERROR_ACCESS_DENIED,
                DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_UNSUPPORTED,
                DXGI_ERROR_WAIT_TIMEOUT, DXGI_MAPPED_RECT, DXGI_MAP_READ, DXGI_OUTDUPL_FRAME_INFO,
            },
        },
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct DxgiOutputIdentity {
    adapter_index: u32,
    output_index: u32,
    monitor_id: u32,
    device_name: String,
    bounds: CaptureRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OutputMonitorMapping {
    output_index: usize,
    monitor_index: usize,
}

#[cfg(windows)]
struct DxgiOutput {
    identity: DxgiOutputIdentity,
    adapter: IDXGIAdapter1,
    output: IDXGIOutput1,
    rotation: DXGI_MODE_ROTATION,
}

/// DXGI Desktop Duplication is available on Windows 8+ with a compatible
/// display driver and an interactive desktop session.
#[cfg(windows)]
pub fn is_supported() -> bool {
    enumerate_dxgi_outputs()
        .map(|outputs| !outputs.is_empty())
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn is_supported() -> bool {
    false
}

/// Uses the same monitor identifiers and virtual coordinates as the WGC
/// backend, allowing the tray overlay to switch backends without remapping.
pub fn monitors() -> Result<Vec<MonitorInfo>, CaptureError> {
    crate::windows_graphics_capture::monitors()
}

/// Captures and stitches every active monitor.
pub fn capture_virtual_desktop() -> Result<CapturedFrame, CaptureError> {
    let started = Instant::now();
    let monitors = monitors()?;
    let bounds = virtual_bounds(&monitors)?;
    let mut frame = capture_region_with_monitors(bounds, monitors, CapturedSource::VirtualDesktop)?;
    frame.elapsed = started.elapsed();
    Ok(frame)
}

/// Captures a region expressed in Windows virtual-desktop coordinates.
pub fn capture_desktop_region(rect: CaptureRect) -> Result<CapturedFrame, CaptureError> {
    let started = Instant::now();
    let rect = validate_rect(rect)?;
    let mut frame = capture_region_with_monitors(rect, monitors()?, CapturedSource::DesktopRegion)?;
    frame.elapsed = started.elapsed();
    Ok(frame)
}

/// Captures one full display by the ID returned from [`monitors`].
pub fn capture_monitor(id: u32) -> Result<CapturedFrame, CaptureError> {
    let started = Instant::now();
    let all = monitors()?;
    let monitor = all.iter().find(|monitor| monitor.id == id).ok_or_else(|| {
        error(
            CaptureErrorKind::InvalidTarget,
            format!("找不到显示器（ID=0x{id:X}）。"),
        )
    })?;
    let bounds = monitor.bounds;
    let mut frame = capture_region_with_monitors(bounds, all, CapturedSource::Monitor(id))?;
    frame.elapsed = started.elapsed();
    Ok(frame)
}

/// Captures the visible desktop pixels inside a top-level window's extended
/// frame bounds. This fallback cannot isolate an occluded window; use WGC for
/// that behavior.
#[cfg(windows)]
pub fn capture_window(hwnd: isize) -> Result<CapturedFrame, CaptureError> {
    let started = Instant::now();
    let bounds = window_bounds(hwnd)?;
    let mut frame =
        capture_region_with_monitors(bounds, monitors()?, CapturedSource::Window(hwnd))?;
    frame.elapsed = started.elapsed();
    Ok(frame)
}

#[cfg(not(windows))]
pub fn capture_window(_hwnd: isize) -> Result<CapturedFrame, CaptureError> {
    Err(unsupported_platform())
}

#[cfg(windows)]
fn capture_region_with_monitors(
    rect: CaptureRect,
    monitors: Vec<MonitorInfo>,
    source: CapturedSource,
) -> Result<CapturedFrame, CaptureError> {
    let _capture_guard = CAPTURE_LOCK.lock().map_err(|_| {
        error(
            CaptureErrorKind::Backend,
            "DXGI 截图锁已损坏，请重新启动截图进程。",
        )
    })?;
    let rect = validate_rect(rect)?;
    let intersecting: Vec<usize> = monitors
        .iter()
        .enumerate()
        .filter_map(|(index, monitor)| {
            intersection(rect, monitor.bounds)
                .is_some()
                .then_some(index)
        })
        .collect();
    if intersecting.is_empty() {
        return Err(error(
            CaptureErrorKind::InvalidTarget,
            "截图区域没有与任何活动显示器相交。",
        ));
    }

    // Do not infer output identity from enumeration order or dimensions.  A
    // DXGI output supplies both its native HMONITOR and its virtual-desktop
    // coordinates.  The pair must agree with the monitor list before any
    // pixels are composed.  Each output also retains its originating adapter,
    // so output 0 on a second GPU is captured from that GPU rather than being
    // confused with output 0 on the first adapter.
    let dxgi_outputs = enumerate_dxgi_outputs()?;
    let identities: Vec<DxgiOutputIdentity> = dxgi_outputs
        .iter()
        .map(|output| output.identity.clone())
        .collect();
    let mappings = map_dxgi_outputs_to_monitors(&identities, &monitors)?;

    let mut output = RgbaImage::from_pixel(rect.width, rect.height, Rgba([0, 0, 0, 255]));
    let mut captured = vec![false; monitors.len()];

    for mapping in mappings {
        if !intersecting.contains(&mapping.monitor_index) {
            continue;
        }

        let monitor = &monitors[mapping.monitor_index];
        let image = capture_dxgi_output(&dxgi_outputs[mapping.output_index])?;
        if image.width() != monitor.bounds.width || image.height() != monitor.bounds.height {
            return Err(error(
                CaptureErrorKind::Backend,
                format!(
                    "DXGI 输出 {}（显卡 {} / 输出 {}）捕获尺寸 {}×{} 与桌面坐标尺寸 {}×{} 不一致。",
                    dxgi_outputs[mapping.output_index].identity.device_name,
                    dxgi_outputs[mapping.output_index].identity.adapter_index,
                    dxgi_outputs[mapping.output_index].identity.output_index,
                    image.width(),
                    image.height(),
                    monitor.bounds.width,
                    monitor.bounds.height
                ),
            ));
        }
        let overlap = intersection(rect, monitor.bounds).expect("intersection checked above");
        let source_x = (i64::from(overlap.x) - i64::from(monitor.bounds.x)) as u32;
        let source_y = (i64::from(overlap.y) - i64::from(monitor.bounds.y)) as u32;
        let cropped =
            image::imageops::crop_imm(&image, source_x, source_y, overlap.width, overlap.height)
                .to_image();
        let target_x = i64::from(overlap.x) - i64::from(rect.x);
        let target_y = i64::from(overlap.y) - i64::from(rect.y);
        image::imageops::replace(&mut output, &cropped, target_x, target_y);
        captured[mapping.monitor_index] = true;
    }

    let missing: Vec<String> = intersecting
        .iter()
        .filter(|&&monitor_index| !captured[monitor_index])
        .map(|&monitor_index| monitors[monitor_index].name.clone())
        .collect();
    if !missing.is_empty() {
        return Err(error(
            CaptureErrorKind::Backend,
            format!(
                "DXGI 未能捕获全部目标显示器：{}。显示拓扑可能刚刚发生变化，建议重试或回退到系统截图。",
                missing.join("、")
            ),
        ));
    }

    Ok(CapturedFrame {
        image: output,
        origin_x: rect.x,
        origin_y: rect.y,
        source,
        elapsed: Duration::ZERO,
    })
}

#[cfg(not(windows))]
fn capture_region_with_monitors(
    _rect: CaptureRect,
    _monitors: Vec<MonitorInfo>,
    _source: CapturedSource,
) -> Result<CapturedFrame, CaptureError> {
    Err(unsupported_platform())
}

fn map_dxgi_outputs_to_monitors(
    outputs: &[DxgiOutputIdentity],
    monitors: &[MonitorInfo],
) -> Result<Vec<OutputMonitorMapping>, CaptureError> {
    if outputs.is_empty() {
        return Err(error(
            CaptureErrorKind::Unsupported,
            "DXGI Desktop Duplication 没有找到连接到桌面的输出。",
        ));
    }
    if monitors.is_empty() {
        return Err(error(
            CaptureErrorKind::InvalidTarget,
            "没有找到活动显示器。",
        ));
    }

    let mut claimed_monitors = vec![false; monitors.len()];
    let mut mappings = Vec::with_capacity(outputs.len());
    for (output_index, output) in outputs.iter().enumerate() {
        let matches: Vec<usize> = monitors
            .iter()
            .enumerate()
            .filter_map(|(monitor_index, monitor)| {
                (monitor.id == output.monitor_id && monitor.bounds == output.bounds)
                    .then_some(monitor_index)
            })
            .collect();

        let monitor_index = match matches.as_slice() {
            [monitor_index] => *monitor_index,
            [] => {
                let detail = monitors
                    .iter()
                    .find(|monitor| monitor.id == output.monitor_id)
                    .map(|monitor| {
                        format!(
                            "HMONITOR 对应显示器的坐标为 {}，DXGI 报告为 {}",
                            format_rect(monitor.bounds),
                            format_rect(output.bounds)
                        )
                    })
                    .unwrap_or_else(|| {
                        format!("活动显示器列表中没有 HMONITOR=0x{:X}", output.monitor_id)
                    });
                return Err(error(
                    CaptureErrorKind::Backend,
                    format!(
                        "无法验证 DXGI 输出 {}（显卡 {} / 输出 {}）到显示器的映射：{}。",
                        output.device_name, output.adapter_index, output.output_index, detail
                    ),
                ));
            }
            _ => {
                return Err(error(
                    CaptureErrorKind::Backend,
                    format!(
                        "DXGI 输出 {}（HMONITOR=0x{:X}，坐标 {}）匹配到多个活动显示器。",
                        output.device_name,
                        output.monitor_id,
                        format_rect(output.bounds)
                    ),
                ));
            }
        };

        if claimed_monitors[monitor_index] {
            return Err(error(
                CaptureErrorKind::Backend,
                format!(
                    "多个 DXGI 输出映射到同一显示器 {}（HMONITOR=0x{:X}，坐标 {}），无法安全拼接。",
                    monitors[monitor_index].name,
                    monitors[monitor_index].id,
                    format_rect(monitors[monitor_index].bounds)
                ),
            ));
        }
        claimed_monitors[monitor_index] = true;
        mappings.push(OutputMonitorMapping {
            output_index,
            monitor_index,
        });
    }

    let missing: Vec<String> = monitors
        .iter()
        .enumerate()
        .filter(|(index, _)| !claimed_monitors[*index])
        .map(|(_, monitor)| {
            format!(
                "{}（HMONITOR=0x{:X}，坐标 {}）",
                monitor.name,
                monitor.id,
                format_rect(monitor.bounds)
            )
        })
        .collect();
    if !missing.is_empty() {
        return Err(error(
            CaptureErrorKind::Backend,
            format!(
                "以下活动显示器没有对应的 DXGI 输出：{}。显示拓扑可能正在变化。",
                missing.join("、")
            ),
        ));
    }

    Ok(mappings)
}

fn format_rect(rect: CaptureRect) -> String {
    format!("({}, {}) {}×{}", rect.x, rect.y, rect.width, rect.height)
}

#[cfg(windows)]
fn enumerate_dxgi_outputs() -> Result<Vec<DxgiOutput>, CaptureError> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|err| map_windows_dxgi_error("创建 DXGI 工厂失败", err))?;
    let mut outputs = Vec::new();

    for adapter_index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(err) => {
                return Err(map_windows_dxgi_error("枚举 DXGI 显卡适配器失败", err));
            }
        };

        for output_index in 0.. {
            let raw_output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(err) => {
                    return Err(map_windows_dxgi_error(
                        format!("枚举 DXGI 显卡 {adapter_index} 的输出失败"),
                        err,
                    ));
                }
            };
            let desc = unsafe { raw_output.GetDesc() }.map_err(|err| {
                map_windows_dxgi_error(
                    format!("读取 DXGI 显卡 {adapter_index} / 输出 {output_index} 描述失败"),
                    err,
                )
            })?;
            if !desc.AttachedToDesktop.as_bool() {
                continue;
            }

            let desktop = desc.DesktopCoordinates;
            let width = u32::try_from(i64::from(desktop.right) - i64::from(desktop.left)).map_err(
                |_| {
                    error(
                        CaptureErrorKind::Backend,
                        format!("DXGI 显卡 {adapter_index} / 输出 {output_index} 的桌面宽度无效。"),
                    )
                },
            )?;
            let height = u32::try_from(i64::from(desktop.bottom) - i64::from(desktop.top))
                .map_err(|_| {
                    error(
                        CaptureErrorKind::Backend,
                        format!("DXGI 显卡 {adapter_index} / 输出 {output_index} 的桌面高度无效。"),
                    )
                })?;
            let bounds = validate_rect(CaptureRect::new(desktop.left, desktop.top, width, height))?;
            let output = raw_output.cast::<IDXGIOutput1>().map_err(|err| {
                map_windows_dxgi_error(
                    format!(
                        "DXGI 显卡 {adapter_index} / 输出 {output_index} 不支持 Desktop Duplication"
                    ),
                    err,
                )
            })?;

            outputs.push(DxgiOutput {
                identity: DxgiOutputIdentity {
                    adapter_index,
                    output_index,
                    monitor_id: desc.Monitor.0 as u32,
                    device_name: utf16_name(&desc.DeviceName),
                    bounds,
                },
                adapter: adapter.clone(),
                output,
                rotation: desc.Rotation,
            });
        }
    }

    Ok(outputs)
}

#[cfg(windows)]
fn capture_dxgi_output(output: &DxgiOutput) -> Result<RgbaImage, CaptureError> {
    let (device, context) = create_d3d11_device(&output.adapter).map_err(|err| {
        map_windows_dxgi_error(
            format!(
                "为 DXGI 输出 {}（显卡 {} / 输出 {}）创建设备失败",
                output.identity.device_name,
                output.identity.adapter_index,
                output.identity.output_index
            ),
            err,
        )
    })?;
    let duplication = unsafe { output.output.DuplicateOutput(&device) }.map_err(|err| {
        map_windows_dxgi_error(
            format!(
                "复制 DXGI 输出 {}（显卡 {} / 输出 {}）失败",
                output.identity.device_name,
                output.identity.adapter_index,
                output.identity.output_index
            ),
            err,
        )
    })?;

    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;
    unsafe { duplication.AcquireNextFrame(CAPTURE_TIMEOUT_MS, &mut frame_info, &mut resource) }
        .map_err(|err| {
            map_windows_dxgi_error(
                format!(
                    "等待 DXGI 输出 {} 的桌面帧失败",
                    output.identity.device_name
                ),
                err,
            )
        })?;
    let mut acquired_frame = AcquiredFrameGuard::new(&duplication);

    if frame_info.ProtectedContentMaskedOut.as_bool() {
        return Err(error(
            CaptureErrorKind::ProtectedContent,
            format!(
                "DXGI 输出 {} 报告当前画面包含受保护内容，系统已将其遮蔽。",
                output.identity.device_name
            ),
        ));
    }

    let texture = resource
        .ok_or_else(|| error(CaptureErrorKind::Backend, "DXGI 没有返回桌面纹理资源。"))?
        .cast::<ID3D11Texture2D>()
        .map_err(|err| map_windows_dxgi_error("读取 DXGI 桌面纹理失败", err))?;
    let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut texture_desc) };
    if texture_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
        return Err(error(
            CaptureErrorKind::Backend,
            format!(
                "DXGI 输出 {} 返回了不支持的像素格式 {:?}。",
                output.identity.device_name, texture_desc.Format
            ),
        ));
    }

    let mut staging_desc = texture_desc;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    staging_desc.MiscFlags = 0;
    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }.map_err(|err| {
        map_windows_dxgi_error(
            format!(
                "创建 DXGI 输出 {} 的暂存纹理失败",
                output.identity.device_name
            ),
            err,
        )
    })?;
    let staging = staging.ok_or_else(|| {
        error(
            CaptureErrorKind::Backend,
            "D3D11 创建暂存纹理后没有返回纹理对象。",
        )
    })?;
    unsafe { context.CopyResource(&staging, &texture) };
    acquired_frame.release().map_err(|err| {
        map_windows_dxgi_error(
            format!(
                "释放 DXGI 输出 {} 的桌面帧失败",
                output.identity.device_name
            ),
            err,
        )
    })?;

    let surface = staging
        .cast::<IDXGISurface1>()
        .map_err(|err| map_windows_dxgi_error("打开 DXGI 暂存纹理失败", err))?;
    let mut mapped = DXGI_MAPPED_RECT::default();
    unsafe { surface.Map(&mut mapped, DXGI_MAP_READ) }.map_err(|err| {
        map_windows_dxgi_error(
            format!(
                "映射 DXGI 输出 {} 的像素缓冲区失败",
                output.identity.device_name
            ),
            err,
        )
    })?;
    let mut mapped_surface = MappedSurfaceGuard::new(&surface);
    let image = copy_mapped_bgra_surface(
        mapped,
        texture_desc.Width,
        texture_desc.Height,
        output.rotation,
        output.identity.bounds,
    )?;
    mapped_surface.unmap().map_err(|err| {
        map_windows_dxgi_error(
            format!(
                "解除 DXGI 输出 {} 的像素映射失败",
                output.identity.device_name
            ),
            err,
        )
    })?;
    Ok(image)
}

#[cfg(windows)]
fn create_d3d11_device(
    adapter: &IDXGIAdapter1,
) -> WindowsResult<(ID3D11Device, ID3D11DeviceContext)> {
    let adapter: IDXGIAdapter = adapter.cast()?;
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x8000_4005u32 as i32,
        ))),
    }
}

#[cfg(windows)]
fn copy_mapped_bgra_surface(
    mapped: DXGI_MAPPED_RECT,
    source_width: u32,
    source_height: u32,
    rotation: DXGI_MODE_ROTATION,
    desktop_bounds: CaptureRect,
) -> Result<RgbaImage, CaptureError> {
    if mapped.pBits.is_null() || mapped.Pitch <= 0 {
        return Err(error(
            CaptureErrorKind::Backend,
            "DXGI 返回了无效的映射像素地址或行跨度。",
        ));
    }
    let (output_width, output_height) = match rotation {
        DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED => {
            (source_width, source_height)
        }
        DXGI_MODE_ROTATION_ROTATE90 | DXGI_MODE_ROTATION_ROTATE270 => (source_height, source_width),
        DXGI_MODE_ROTATION_ROTATE180 => (source_width, source_height),
        _ => {
            return Err(error(
                CaptureErrorKind::Backend,
                format!("DXGI 返回了未知的显示旋转值 {:?}。", rotation),
            ));
        }
    };
    if output_width != desktop_bounds.width || output_height != desktop_bounds.height {
        return Err(error(
            CaptureErrorKind::Backend,
            format!(
                "DXGI 纹理旋转后为 {}×{}，但输出桌面坐标为 {}。",
                output_width,
                output_height,
                format_rect(desktop_bounds)
            ),
        ));
    }

    let pitch = usize::try_from(mapped.Pitch)
        .map_err(|_| error(CaptureErrorKind::Backend, "DXGI 行跨度无法表示。"))?;
    let source_row_bytes = usize::try_from(source_width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| error(CaptureErrorKind::Backend, "DXGI 源图像行大小溢出。"))?;
    if pitch < source_row_bytes {
        return Err(error(
            CaptureErrorKind::Backend,
            format!("DXGI 行跨度 {pitch} 小于像素行大小 {source_row_bytes}。"),
        ));
    }
    let source_len = pitch
        .checked_mul(source_height as usize)
        .ok_or_else(|| error(CaptureErrorKind::Backend, "DXGI 源图像缓冲区大小溢出。"))?;
    let output_len = (output_width as usize)
        .checked_mul(output_height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| error(CaptureErrorKind::Backend, "DXGI 输出图像缓冲区大小溢出。"))?;
    let source = unsafe { std::slice::from_raw_parts(mapped.pBits, source_len) };
    let mut rgba = vec![0u8; output_len];

    for y in 0..output_height {
        for x in 0..output_width {
            let (source_x, source_y) = match rotation {
                DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED => (x, y),
                DXGI_MODE_ROTATION_ROTATE90 => (y, source_height - 1 - x),
                DXGI_MODE_ROTATION_ROTATE180 => (source_width - 1 - x, source_height - 1 - y),
                DXGI_MODE_ROTATION_ROTATE270 => (source_width - 1 - y, x),
                _ => unreachable!("rotation validated above"),
            };
            let source_offset = source_y as usize * pitch + source_x as usize * 4;
            let target_offset = (y as usize * output_width as usize + x as usize) * 4;
            rgba[target_offset] = source[source_offset + 2];
            rgba[target_offset + 1] = source[source_offset + 1];
            rgba[target_offset + 2] = source[source_offset];
            rgba[target_offset + 3] = 255;
        }
    }

    RgbaImage::from_raw(output_width, output_height, rgba).ok_or_else(|| {
        error(
            CaptureErrorKind::Backend,
            "无法从 DXGI 像素缓冲区创建图像。",
        )
    })
}

#[cfg(windows)]
fn utf16_name(value: &[u16]) -> String {
    let end = value.iter().position(|&ch| ch == 0).unwrap_or(value.len());
    let name = String::from_utf16_lossy(&value[..end]);
    if name.is_empty() {
        "未命名输出".to_string()
    } else {
        name
    }
}

#[cfg(windows)]
struct AcquiredFrameGuard<'a> {
    duplication: &'a IDXGIOutputDuplication,
    active: bool,
}

#[cfg(windows)]
impl<'a> AcquiredFrameGuard<'a> {
    fn new(duplication: &'a IDXGIOutputDuplication) -> Self {
        Self {
            duplication,
            active: true,
        }
    }

    fn release(&mut self) -> WindowsResult<()> {
        if self.active {
            self.active = false;
            unsafe { self.duplication.ReleaseFrame() }
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
impl Drop for AcquiredFrameGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = unsafe { self.duplication.ReleaseFrame() };
        }
    }
}

#[cfg(windows)]
struct MappedSurfaceGuard<'a> {
    surface: &'a IDXGISurface1,
    active: bool,
}

#[cfg(windows)]
impl<'a> MappedSurfaceGuard<'a> {
    fn new(surface: &'a IDXGISurface1) -> Self {
        Self {
            surface,
            active: true,
        }
    }

    fn unmap(&mut self) -> WindowsResult<()> {
        if self.active {
            self.active = false;
            unsafe { self.surface.Unmap() }
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
impl Drop for MappedSurfaceGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = unsafe { self.surface.Unmap() };
        }
    }
}

#[cfg(windows)]
fn window_bounds(hwnd: isize) -> Result<CaptureRect, CaptureError> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindow};

    if hwnd == 0 {
        return Err(error(CaptureErrorKind::InvalidTarget, "窗口句柄为空。"));
    }
    let hwnd = HWND(hwnd as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err(error(
            CaptureErrorKind::InvalidTarget,
            "目标窗口已经关闭或句柄无效。",
        ));
    }
    if unsafe { IsIconic(hwnd).as_bool() } {
        return Err(error(
            CaptureErrorKind::InvalidTarget,
            "目标窗口已最小化，DXGI 无法获得其可见桌面区域。",
        ));
    }

    let mut rect = RECT::default();
    let dwm_result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if dwm_result.is_err() {
        unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|err| {
            let code = err.code().0;
            error_with_hresult(
                CaptureErrorKind::InvalidTarget,
                format!("读取目标窗口矩形失败：{err}"),
                code,
            )
        })?;
    }

    let width = u32::try_from(i64::from(rect.right) - i64::from(rect.left))
        .map_err(|_| error(CaptureErrorKind::InvalidTarget, "目标窗口宽度无效。"))?;
    let height = u32::try_from(i64::from(rect.bottom) - i64::from(rect.top))
        .map_err(|_| error(CaptureErrorKind::InvalidTarget, "目标窗口高度无效。"))?;
    validate_rect(CaptureRect::new(rect.left, rect.top, width, height))
}

fn validate_rect(rect: CaptureRect) -> Result<CaptureRect, CaptureError> {
    if rect.width == 0 || rect.height == 0 {
        return Err(error(
            CaptureErrorKind::InvalidTarget,
            "截图区域的宽度和高度必须大于 0。",
        ));
    }
    let pixels = u64::from(rect.width)
        .checked_mul(u64::from(rect.height))
        .ok_or_else(|| error(CaptureErrorKind::InvalidTarget, "截图区域尺寸溢出。"))?;
    if pixels > MAX_CAPTURE_PIXELS {
        return Err(error(
            CaptureErrorKind::InvalidTarget,
            format!(
                "截图区域过大（{}×{}），已拒绝分配图像缓冲区。",
                rect.width, rect.height
            ),
        ));
    }
    Ok(rect)
}

fn virtual_bounds(monitors: &[MonitorInfo]) -> Result<CaptureRect, CaptureError> {
    let first = monitors
        .first()
        .ok_or_else(|| error(CaptureErrorKind::InvalidTarget, "没有找到活动显示器。"))?;
    let mut left = i64::from(first.bounds.x);
    let mut top = i64::from(first.bounds.y);
    let mut right = rect_right(first.bounds);
    let mut bottom = rect_bottom(first.bounds);
    for monitor in &monitors[1..] {
        left = left.min(i64::from(monitor.bounds.x));
        top = top.min(i64::from(monitor.bounds.y));
        right = right.max(rect_right(monitor.bounds));
        bottom = bottom.max(rect_bottom(monitor.bounds));
    }
    let width = u32::try_from(right - left)
        .map_err(|_| error(CaptureErrorKind::InvalidTarget, "虚拟桌面宽度溢出。"))?;
    let height = u32::try_from(bottom - top)
        .map_err(|_| error(CaptureErrorKind::InvalidTarget, "虚拟桌面高度溢出。"))?;
    validate_rect(CaptureRect::new(left as i32, top as i32, width, height))
}

fn intersection(left: CaptureRect, right: CaptureRect) -> Option<CaptureRect> {
    let x1 = i64::from(left.x).max(i64::from(right.x));
    let y1 = i64::from(left.y).max(i64::from(right.y));
    let x2 = rect_right(left).min(rect_right(right));
    let y2 = rect_bottom(left).min(rect_bottom(right));
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some(CaptureRect::new(
        x1 as i32,
        y1 as i32,
        (x2 - x1) as u32,
        (y2 - y1) as u32,
    ))
}

fn rect_right(rect: CaptureRect) -> i64 {
    i64::from(rect.x) + i64::from(rect.width)
}

fn rect_bottom(rect: CaptureRect) -> i64 {
    i64::from(rect.y) + i64::from(rect.height)
}

#[cfg(windows)]
fn map_windows_dxgi_error(
    context: impl AsRef<str>,
    error_value: windows::core::Error,
) -> CaptureError {
    let code = error_value.code();
    if code == DXGI_ERROR_ACCESS_DENIED {
        error_with_hresult(
            CaptureErrorKind::ProtectedContent,
            format!(
                "{}：DXGI 拒绝访问显示输出，画面可能包含受保护内容。",
                context.as_ref()
            ),
            code.0,
        )
    } else if code == DXGI_ERROR_WAIT_TIMEOUT {
        error_with_hresult(
            CaptureErrorKind::Timeout,
            format!("{}：等待桌面帧超时。", context.as_ref()),
            code.0,
        )
    } else if code == DXGI_ERROR_ACCESS_LOST {
        error_with_hresult(
            CaptureErrorKind::Backend,
            format!(
                "{}：DXGI 输出访问已丢失，显示模式可能刚刚发生变化。",
                context.as_ref()
            ),
            code.0,
        )
    } else if code == DXGI_ERROR_UNSUPPORTED || code == DXGI_ERROR_NOT_FOUND {
        error_with_hresult(
            CaptureErrorKind::Unsupported,
            format!("{}：当前显示驱动不支持该 DXGI 操作。", context.as_ref()),
            code.0,
        )
    } else {
        error_with_hresult(
            CaptureErrorKind::Backend,
            format!("{}：{}", context.as_ref(), error_value),
            code.0,
        )
    }
}

fn error(kind: CaptureErrorKind, message: impl Into<String>) -> CaptureError {
    CaptureError {
        kind,
        message: message.into(),
        hresult: None,
    }
}

fn error_with_hresult(
    kind: CaptureErrorKind,
    message: impl Into<String>,
    hresult: i32,
) -> CaptureError {
    CaptureError {
        kind,
        message: message.into(),
        hresult: Some(hresult),
    }
}

#[cfg(not(windows))]
fn unsupported_platform() -> CaptureError {
    error(
        CaptureErrorKind::Unsupported,
        "DXGI Desktop Duplication 仅支持 Windows 8 及更高版本。",
    )
}
