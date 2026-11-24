//! Safe Rust FFI wrapper over `rapidocrlite.dll`'s stable C ABI.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::Arc;

pub const RPOCR_OK: c_int = 0;
pub const RPOCR_BACKEND_CPU: c_int = 0;
pub const RPOCR_MODEL_PPOCR: c_int = 1;

pub const RPOCR_TASK_DET: c_int = 1;
pub const RPOCR_TASK_REC: c_int = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RpocrOptions {
    pub struct_size: c_int,
    pub backend: c_int,
    pub det_threshold: f32,
    pub det_box_threshold: f32,
    pub det_unclip_ratio: f32,
    pub det_limit_side_len: c_int,
    pub rec_height: c_int,
    pub rec_max_width: c_int,
    pub rec_batch_size: c_int,
    pub rec_batch_width_bucket: c_int,
    pub rec_parallelism: c_int,
    pub hybrid_graph_parallelism: c_int,
    pub det_batch_size: c_int,
    pub batch_preprocess_parallelism: c_int,
    pub image_batch_parallelism: c_int,
    pub det_limit_type: c_int,
    pub det_mean: [f32; 3],
    pub det_std: [f32; 3],
    pub rec_mean: [f32; 3],
    pub rec_std: [f32; 3],
    pub det_use_dilation: c_int,
    pub det_max_candidates: c_int,
    pub vulkan_device_index: c_int,
    pub rec_fast_mode: c_int,
    pub model_type: c_int,
    pub use_cls: c_int,
    pub cls_thresh: f32,
    pub cls_batch_size: c_int,
    pub cls_height: c_int,
    pub cls_width: c_int,
    pub cls_crop_long_lines: c_int,
    pub cls_model: *const c_char,
}

#[repr(C)]
struct RpocrImage {
    width: c_int,
    height: c_int,
    rgb: *const u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RpocrPoint {
    x: f32,
    y: f32,
}

#[repr(C)]
struct RpocrTextBox {
    text: *const c_char,
    confidence: f32,
    bbox_x: c_int,
    bbox_y: c_int,
    bbox_w: c_int,
    bbox_h: c_int,
    quad: [RpocrPoint; 4],
}

#[repr(C)]
struct RpocrResult {
    items: *mut RpocrTextBox,
    count: usize,
}

#[derive(Clone, Debug)]
pub struct RawOcrBox {
    pub text: String,
    pub score: f32,
    pub quad: [[f32; 2]; 4],
}

type FnOptionsInit = unsafe extern "C" fn(*mut RpocrOptions);
type FnOptionsInitEx = unsafe extern "C" fn(*mut RpocrOptions, usize);
type FnCreate = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *const RpocrOptions,
) -> *mut c_void;
type FnDestroy = unsafe extern "C" fn(*mut c_void);
type FnSetDetThresholds = unsafe extern "C" fn(*mut c_void, f32, f32, f32) -> c_int;
type FnRecognize = unsafe extern "C" fn(*mut c_void, *const RpocrImage, *mut RpocrResult) -> c_int;
type FnRun = unsafe extern "C" fn(*mut c_void, *const RpocrImage, c_int, *mut RpocrResult) -> c_int;
type FnResultFree = unsafe extern "C" fn(*mut RpocrResult);
type FnLastError = unsafe extern "C" fn() -> *const c_char;
type FnResizeLinearRgb =
    unsafe extern "C" fn(*const u8, c_int, c_int, *mut u8, c_int, c_int) -> c_int;

pub struct RapidOcrDll {
    #[cfg(windows)]
    module: isize,
    options_init: Option<FnOptionsInit>,
    options_init_ex: Option<FnOptionsInitEx>,
    create: FnCreate,
    destroy: FnDestroy,
    set_det_thresholds: FnSetDetThresholds,
    recognize: FnRecognize,
    run: Option<FnRun>,
    result_free: FnResultFree,
    last_error: FnLastError,
    resize_linear_rgb: Option<FnResizeLinearRgb>,
}

// The loaded DLL handle and exported function pointers are immutable once loaded.
unsafe impl Send for RapidOcrDll {}
unsafe impl Sync for RapidOcrDll {}

impl Drop for RapidOcrDll {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.module != 0 {
            unsafe {
                windows_sys::Win32::Foundation::FreeLibrary(self.module);
            }
        }
    }
}

impl RapidOcrDll {
    #[cfg(windows)]
    pub fn load(dll_path: &Path) -> Result<Arc<Self>, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::FreeLibrary;
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

        if !dll_path.is_file() {
            return Err(format!(
                "RapidOCRLite native library not found: {}",
                dll_path.display()
            ));
        }

        let wide: Vec<u16> = dll_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let module = unsafe { LoadLibraryW(wide.as_ptr()) };
        if module == 0 {
            return Err(format!(
                "failed to load RapidOCRLite library {}: {}",
                dll_path.display(),
                std::io::Error::last_os_error()
            ));
        }

        unsafe fn load_sym<T: Copy>(
            module: isize,
            name: &'static [u8],
            dll_path: &Path,
        ) -> Result<T, String> {
            let Some(ptr) = GetProcAddress(module, name.as_ptr()) else {
                let sym = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]);
                return Err(format!(
                    "RapidOCRLite library {} is missing required symbol {}",
                    dll_path.display(),
                    sym
                ));
            };
            Ok(std::mem::transmute_copy(&ptr))
        }

        unsafe fn load_opt_sym<T: Copy>(module: isize, name: &'static [u8]) -> Option<T> {
            let ptr = GetProcAddress(module, name.as_ptr())?;
            Some(std::mem::transmute_copy(&ptr))
        }

        let loaded = (|| -> Result<Self, String> {
            unsafe {
                let options_init_ex: Option<FnOptionsInitEx> =
                    load_opt_sym(module, b"rpocr_options_init_ex\0");
                let options_init: Option<FnOptionsInit> =
                    load_opt_sym(module, b"rpocr_options_init\0");
                if options_init_ex.is_none() && options_init.is_none() {
                    return Err(format!(
                        "RapidOCRLite library {} is missing rpocr_options_init_ex / rpocr_options_init",
                        dll_path.display()
                    ));
                }
                Ok(Self {
                    module,
                    options_init,
                    options_init_ex,
                    create: load_sym(module, b"rpocr_create\0", dll_path)?,
                    destroy: load_sym(module, b"rpocr_destroy\0", dll_path)?,
                    set_det_thresholds: load_sym(module, b"rpocr_set_det_thresholds\0", dll_path)?,
                    recognize: load_sym(module, b"rpocr_recognize\0", dll_path)?,
                    run: load_opt_sym(module, b"rpocr_run\0"),
                    result_free: load_sym(module, b"rpocr_result_free\0", dll_path)?,
                    last_error: load_sym(module, b"rpocr_last_error\0", dll_path)?,
                    resize_linear_rgb: load_opt_sym(module, b"rpocr_resize_linear_rgb\0"),
                })
            }
        })();

        match loaded {
            Ok(dll) => Ok(Arc::new(dll)),
            Err(err) => {
                unsafe {
                    FreeLibrary(module);
                }
                Err(err)
            }
        }
    }

    #[cfg(not(windows))]
    pub fn load(_dll_path: &Path) -> Result<Arc<Self>, String> {
        Err("RapidOCRLite native FFI is only supported on Windows.".to_string())
    }

    pub fn default_options(&self) -> RpocrOptions {
        let mut opts = unsafe { std::mem::zeroed::<RpocrOptions>() };
        unsafe {
            if let Some(init_ex) = self.options_init_ex {
                init_ex(&mut opts, std::mem::size_of::<RpocrOptions>());
            } else if let Some(init) = self.options_init {
                init(&mut opts);
            }
        }
        opts
    }

    pub fn last_error_message(&self) -> String {
        let ptr = unsafe { (self.last_error)() };
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .trim()
                .to_string()
        }
    }

    pub fn resize_linear_rgb(
        &self,
        src: &[u8],
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
    ) -> Option<Vec<u8>> {
        let resize = self.resize_linear_rgb?;
        let expected_src = (src_w as usize)
            .checked_mul(src_h as usize)?
            .checked_mul(3)?;
        if src.len() < expected_src || dst_w == 0 || dst_h == 0 {
            return None;
        }
        let dst_len = (dst_w as usize)
            .checked_mul(dst_h as usize)?
            .checked_mul(3)?;
        let mut out = vec![0u8; dst_len];
        let status = unsafe {
            resize(
                src.as_ptr(),
                src_w as c_int,
                src_h as c_int,
                out.as_mut_ptr(),
                dst_w as c_int,
                dst_h as c_int,
            )
        };
        if status == RPOCR_OK {
            Some(out)
        } else {
            None
        }
    }
}

pub struct NativeRapidOcr {
    dll: Arc<RapidOcrDll>,
    handle: *mut c_void,
}

// Each handle is owned exclusively inside the service mutex.
unsafe impl Send for NativeRapidOcr {}

impl Drop for NativeRapidOcr {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                (self.dll.destroy)(self.handle);
            }
            self.handle = ptr::null_mut();
        }
    }
}

impl NativeRapidOcr {
    pub fn create(
        dll: Arc<RapidOcrDll>,
        det_path: &Path,
        rec_path: &Path,
        dict_path: &Path,
        options: &RpocrOptions,
    ) -> Result<Self, String> {
        let det = path_to_utf8_cstring(det_path, "detector model")?;
        let rec = path_to_utf8_cstring(rec_path, "recognizer model")?;
        let dict = path_to_utf8_cstring(dict_path, "dictionary")?;
        let handle = unsafe {
            (dll.create)(
                det.as_ptr(),
                rec.as_ptr(),
                dict.as_ptr(),
                options as *const _,
            )
        };
        if handle.is_null() {
            let reason = dll.last_error_message();
            return Err(if reason.is_empty() {
                "rpocr_create failed".to_string()
            } else {
                format!("rpocr_create failed: {reason}")
            });
        }
        Ok(Self { dll, handle })
    }

    pub fn set_det_thresholds(
        &mut self,
        det_threshold: f32,
        box_threshold: f32,
        unclip_ratio: f32,
    ) -> Result<(), String> {
        let status = unsafe {
            (self.dll.set_det_thresholds)(self.handle, det_threshold, box_threshold, unclip_ratio)
        };
        if status != RPOCR_OK {
            let reason = self.dll.last_error_message();
            return Err(if reason.is_empty() {
                format!("rpocr_set_det_thresholds failed with status {status}")
            } else {
                format!("rpocr_set_det_thresholds failed: {reason}")
            });
        }
        Ok(())
    }

    pub fn run_rgb(
        &mut self,
        rgb: &[u8],
        width: u32,
        height: u32,
        tasks: c_int,
    ) -> Result<Vec<RawOcrBox>, String> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or_else(|| "image byte size overflow".to_string())?;
        if rgb.len() < expected || width == 0 || height == 0 {
            return Err(format!(
                "invalid RGB image buffer: {}x{}, bytes={}",
                width,
                height,
                rgb.len()
            ));
        }

        let image = RpocrImage {
            width: width as c_int,
            height: height as c_int,
            rgb: rgb.as_ptr(),
        };
        let mut result = RpocrResult {
            items: ptr::null_mut(),
            count: 0,
        };

        let status = if tasks == (RPOCR_TASK_DET | RPOCR_TASK_REC) {
            unsafe { (self.dll.recognize)(self.handle, &image, &mut result) }
        } else if let Some(run_fn) = self.dll.run {
            unsafe { run_fn(self.handle, &image, tasks, &mut result) }
        } else {
            return Err("rapidocrlite.dll does not export rpocr_run".to_string());
        };

        struct ResultGuard<'a> {
            dll: &'a RapidOcrDll,
            result: &'a mut RpocrResult,
        }
        impl Drop for ResultGuard<'_> {
            fn drop(&mut self) {
                unsafe {
                    (self.dll.result_free)(self.result);
                }
            }
        }
        let guard = ResultGuard {
            dll: &self.dll,
            result: &mut result,
        };

        if status != RPOCR_OK {
            let reason = self.dll.last_error_message();
            return Err(if reason.is_empty() {
                format!("rapidocrlite inference failed with status {status}")
            } else {
                format!("rapidocrlite inference failed: {reason}")
            });
        }

        let mut out = Vec::with_capacity(guard.result.count);
        if !guard.result.items.is_null() && guard.result.count > 0 {
            let items =
                unsafe { std::slice::from_raw_parts(guard.result.items, guard.result.count) };
            for item in items {
                let text = if item.text.is_null() {
                    String::new()
                } else {
                    unsafe { CStr::from_ptr(item.text) }
                        .to_string_lossy()
                        .into_owned()
                };
                out.push(RawOcrBox {
                    text,
                    score: item.confidence,
                    quad: [
                        [item.quad[0].x, item.quad[0].y],
                        [item.quad[1].x, item.quad[1].y],
                        [item.quad[2].x, item.quad[2].y],
                        [item.quad[3].x, item.quad[3].y],
                    ],
                });
            }
        }
        Ok(out)
    }
}

fn path_to_utf8_cstring(path: &Path, label: &str) -> Result<CString, String> {
    let text = path.to_string_lossy();
    CString::new(text.as_bytes()).map_err(|_| format!("invalid {label} path containing NUL byte"))
}
