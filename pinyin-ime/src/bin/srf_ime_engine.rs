#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod win {
    use std::thread;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    const STARTUP_WARMUP_DELAY_ARG: &str = "--startup-warmup-delay-ms";
    const NO_WARMUP_ARG: &str = "--no-warmup";
    const PIPE_NAME_ARG: &str = "--pipe-name";
    const MUTEX_NAME_ARG: &str = "--mutex-name";
    const LEXICON_DIR_ARG: &str = "--lexicon-dir";
    const INSTALL_HEALTH_CHECK_ARG: &str = "--install-health-check";
    const HEALTH_CHECK_ARG: &str = "--health-check";
    const PROBE_ARG: &str = "--probe";

    pub fn run() {
        unsafe {
            apply_ipc_name_args();
            if let Some(probe_input) = health_check_probe_input() {
                apply_install_root_ipc_names();
                std::process::exit(run_health_check_probe(&probe_input));
            }
            // A login-started engine has no TSF process available to pass the
            // per-install IPC names. Derive the same names from our install
            // root so it is immediately reusable when the first TIP loads.
            apply_install_root_ipc_names();
            let mutex = create_single_instance_mutex();
            if mutex == 0 {
                return;
            }

            pinyin_ime::ipc_service::start_engine_service();
            if !std::env::args().any(|arg| arg == NO_WARMUP_ARG) {
                warmup_engine_before_serving(startup_warmup_delay());
            }

            loop {
                thread::park_timeout(Duration::from_secs(3600));
            }
        }
    }

    fn health_check_probe_input() -> Option<String> {
        let mut args = std::env::args().skip(1).peekable();
        let mut enabled = false;
        let mut probe = "nihao".to_string();
        while let Some(arg) = args.next() {
            if arg == INSTALL_HEALTH_CHECK_ARG || arg == HEALTH_CHECK_ARG {
                enabled = true;
                continue;
            }
            let probe_value = if let Some(value) = arg.strip_prefix(&format!("{PROBE_ARG}=")) {
                Some(value.to_string())
            } else if arg == PROBE_ARG {
                args.next()
            } else {
                None
            };
            if let Some(value) = probe_value {
                enabled = true;
                probe = value;
            }
        }
        if enabled {
            Some(probe)
        } else {
            None
        }
    }

    fn run_health_check_probe(input: &str) -> i32 {
        let direct = pinyin_ime::ipc_service::probe_shared_engine(input);
        let pipe = if direct.is_ok() {
            pinyin_ime::ipc_service::probe_shared_engine_via_pipe(input)
        } else {
            Err("skipped because direct probe failed".to_string())
        };
        match (&direct, &pipe) {
            (Ok(direct_count), Ok(pipe_count)) => {
                println!(
                    "event=install_health_check_probe status=ok probe={} direct=ok pipe=ok direct_candidates={} pipe_candidates={}",
                    log_value(input),
                    direct_count,
                    pipe_count
                );
                0
            }
            _ => {
                let direct_status = if direct.is_ok() { "ok" } else { "failed" };
                let pipe_status = if pipe.is_ok() { "ok" } else { "failed" };
                let direct_reason = direct
                    .as_ref()
                    .err()
                    .map(|err| log_value(err))
                    .unwrap_or_else(|| "none".to_string());
                let pipe_reason = pipe
                    .as_ref()
                    .err()
                    .map(|err| log_value(err))
                    .unwrap_or_else(|| "none".to_string());
                eprintln!(
                    "event=install_health_check_probe status=failed probe={} direct={} pipe={} direct_reason={} pipe_reason={}",
                    log_value(input),
                    direct_status,
                    pipe_status,
                    direct_reason,
                    pipe_reason
                );
                2
            }
        }
    }

    fn apply_install_root_ipc_names() {
        if std::env::var("SRF_ENGINE_PIPE_NAME").is_ok()
            && std::env::var("SRF_ENGINE_MUTEX_NAME").is_ok()
        {
            return;
        }
        let Some(install_root) = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|dir| dir.to_path_buf()))
        else {
            return;
        };
        let install_root = pinyin_ime::shared_rules::normalize_install_root(&install_root);
        if std::env::var("SRF_ENGINE_PIPE_NAME").is_err() {
            std::env::set_var(
                "SRF_ENGINE_PIPE_NAME",
                pinyin_ime::shared_rules::engine_pipe_name_for_install_root(&install_root),
            );
        }
        if std::env::var("SRF_ENGINE_MUTEX_NAME").is_err() {
            std::env::set_var(
                "SRF_ENGINE_MUTEX_NAME",
                pinyin_ime::shared_rules::engine_mutex_name_for_install_root(&install_root),
            );
        }
    }

    fn log_value(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars().take(160) {
            if ch.is_whitespace() || matches!(ch, '=' | ';' | ',') {
                out.push('_');
            } else {
                out.push(ch);
            }
        }
        if out.is_empty() {
            "(none)".to_string()
        } else {
            out
        }
    }

    unsafe fn create_single_instance_mutex() -> HANDLE {
        let mutex_name = pinyin_ime::ipc_service::engine_mutex_name_from_env();
        let name = wide(&mutex_name);
        let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if mutex == 0 {
            return 0;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return 0;
        }
        mutex
    }

    fn apply_ipc_name_args() {
        let mut args = std::env::args().skip(1).peekable();
        while let Some(arg) = args.next() {
            let pipe_name = if let Some(value) = arg.strip_prefix(&format!("{PIPE_NAME_ARG}=")) {
                Some(value.to_string())
            } else if arg == PIPE_NAME_ARG {
                args.next()
            } else {
                None
            };
            if let Some(value) = pipe_name {
                std::env::set_var("SRF_ENGINE_PIPE_NAME", value);
                continue;
            }

            let mutex_name = if let Some(value) = arg.strip_prefix(&format!("{MUTEX_NAME_ARG}=")) {
                Some(value.to_string())
            } else if arg == MUTEX_NAME_ARG {
                args.next()
            } else {
                None
            };
            if let Some(value) = mutex_name {
                std::env::set_var("SRF_ENGINE_MUTEX_NAME", value);
                continue;
            }

            let lexicon_dir = if let Some(value) = arg.strip_prefix(&format!("{LEXICON_DIR_ARG}="))
            {
                Some(value.to_string())
            } else if arg == LEXICON_DIR_ARG {
                args.next()
            } else {
                None
            };
            if let Some(value) = lexicon_dir {
                std::env::set_var("SRF_LEXICON_DIR", value);
            }
        }
    }

    fn startup_warmup_delay() -> Option<Duration> {
        let mut args = std::env::args().skip(1).peekable();
        while let Some(arg) = args.next() {
            let value =
                if let Some(value) = arg.strip_prefix(&format!("{STARTUP_WARMUP_DELAY_ARG}=")) {
                    Some(value.to_string())
                } else if arg == STARTUP_WARMUP_DELAY_ARG {
                    args.next()
                } else {
                    None
                };

            if let Some(value) = value {
                if let Ok(ms) = value.parse::<u64>() {
                    return Some(Duration::from_millis(ms.min(60_000)));
                }
            }
        }
        None
    }

    fn warmup_engine_before_serving(delay: Option<Duration>) {
        if let Some(delay) = delay.filter(|delay| !delay.is_zero()) {
            thread::sleep(delay);
        }
        pinyin_ime::ipc_service::warmup_shared_engine_sync();
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
fn main() {
    pinyin_ime::windows_security::apply_process_hardening();
    win::run();
}
