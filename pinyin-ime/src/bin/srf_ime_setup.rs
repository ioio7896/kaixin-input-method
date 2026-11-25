#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod config_migrate {
    include!("srf_ime_setup/config_migrate.rs");
}

#[cfg(windows)]
mod manifest {
    include!("srf_ime_setup/manifest.rs");
}

#[cfg(windows)]
mod win_acl {
    include!("srf_ime_setup/win_acl.rs");
}

#[cfg(windows)]
mod win_system {
    include!("srf_ime_setup/win_system.rs");
}

#[cfg(windows)]
mod cli {
    use crate::config_migrate::migrate_user_config;
    use crate::manifest::{
        normalize_lexical, relative_to_root, resolve_runtime_payload_arch_root,
        resolve_runtime_payload_root, resolve_successful_runtime_payload_root, sha256_file_hex,
        verify_package_hash_manifest, write_runtime_payload_markers, RUNTIME_PAYLOAD_MANIFEST_NAME,
        RUNTIME_PAYLOAD_ROOT_NAME,
    };
    use crate::win_acl::set_install_directory_access;
    use crate::win_system::{
        add_tip_to_current_user_language_list, assert_trusted_install_root, check_com_registration,
        check_profile_registry, cleanup_old_runtime_payloads, clear_transient_user_state,
        current_user_sid_string, default_user_install_root, ensure_ctfmon_running,
        find_desktop_user_sid, format_health_value, invoke_desktop_user_task,
        is_current_process_elevated, is_machine_install_root, is_tip_in_user_language_list,
        machine_install_root, probe_engine_health, register_hkcu_runonce_cleanup,
        remove_kaixin_user_data, remove_machine_state_root, remove_tip_from_loaded_user_registry,
        remove_tray_run_entries, remove_tree_no_follow, remove_uninstall_registry_entry,
        remove_user_com_overrides, restore_microsoft_pinyin, run_regsvr32,
        schedule_pending_delete_tree, schedule_self_deferred_dir_delete, start_tray_helper,
        stop_running_helpers, update_text_input_services, user_state_root,
        write_install_maintenance_flag, write_tray_run_entry, RegsvrMode, SIMPLIFIED_CHINESE_TIP,
    };
    use chrono::Local;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};

    struct Logger {
        path: Option<PathBuf>,
        echo_stdout: bool,
    }

    impl Logger {
        fn new(path: Option<PathBuf>, echo_stdout: bool) -> Self {
            Self { path, echo_stdout }
        }

        fn log(&mut self, msg: &str) {
            if self.echo_stdout {
                println!("{msg}");
            }
            if let Some(ref p) = self.path {
                if let Some(parent) = p.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
                    let stamp = Local::now().format("%Y-%m-%d %H:%M:%S");
                    let _ = writeln!(f, "{stamp} {msg}");
                }
            }
        }
    }

    fn exe_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn take_flag_value(args: &[String], idx: &mut usize, flag: &str) -> Result<String, String> {
        *idx += 1;
        args.get(*idx)
            .cloned()
            .ok_or_else(|| format!("missing value for {flag}"))
    }

    fn core_manifest_files(root: &Path, dll_x64: &Path, dll_x86: &Path) -> Vec<String> {
        vec![
            RUNTIME_PAYLOAD_MANIFEST_NAME.to_string(),
            "VERSION".to_string(),
            "srf_ime_setup.exe".to_string(),
            "srf_ime_desktop.exe".to_string(),
            "srf_ime_engine.exe".to_string(),
            relative_to_root(root, dll_x64),
            relative_to_root(root, dll_x86),
        ]
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
        fs::create_dir_all(dst).map_err(|e| format!("create dir {}: {e}", dst.display()))?;
        let entries = fs::read_dir(src).map_err(|e| format!("read dir {}: {e}", src.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read entry {}: {e}", src.display()))?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if from.is_dir() {
                copy_dir_recursive(&from, &to)?;
            } else {
                fs::copy(&from, &to)
                    .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))?;
            }
        }
        Ok(())
    }

    fn copy_package_to_install_root(
        source_root: &Path,
        install_root: &Path,
        logger: &mut Logger,
    ) -> Result<(), String> {
        let src = normalize_lexical(source_root);
        let dst = normalize_lexical(install_root);
        if src
            .to_string_lossy()
            .eq_ignore_ascii_case(&dst.to_string_lossy())
        {
            logger.log("SKIP: install root equals source root; not copying package files");
            return Ok(());
        }
        assert_trusted_install_root(&dst)?;
        stop_running_helpers();
        if dst.is_dir() {
            if let Ok(entries) = fs::read_dir(&dst) {
                for entry in entries.flatten() {
                    let _ = remove_tree_no_follow(&entry.path());
                }
            }
        }
        fs::create_dir_all(&dst)
            .map_err(|e| format!("create install root {}: {e}", dst.display()))?;
        copy_dir_recursive(&src, &dst)?;
        logger.log("OK: copied package files");
        Ok(())
    }

    fn try_rollback_runtime(
        install_root: &Path,
        previous_payload: Option<&Path>,
        machine: bool,
        reg_log: &Path,
        logger: &mut Logger,
    ) {
        let Some(prev) = previous_payload else {
            logger.log("WARN: no previous successful runtime payload available for rollback");
            return;
        };
        if !prev.is_dir() {
            logger.log("WARN: previous successful runtime payload directory missing for rollback");
            return;
        }
        let x64_dir = if prev.join("x64").is_dir() {
            prev.join("x64")
        } else {
            prev.to_path_buf()
        };
        let dll_x64 = x64_dir.join("srf_tsf_tip.dll");
        let dll_x86 = prev.join("x86").join("srf_tsf_tip.dll");
        if !dll_x64.is_file() || !dll_x86.is_file() {
            logger.log("WARN: previous runtime payload missing x64/x86 DLL for rollback");
            return;
        }
        let _ = write_runtime_payload_markers(install_root, prev);
        let r1 = run_regsvr32(
            "x64",
            &dll_x64,
            RegsvrMode::Register { machine },
            Some(reg_log),
        );
        let r2 = run_regsvr32(
            "x86",
            &dll_x86,
            RegsvrMode::Register { machine },
            Some(reg_log),
        );
        if r1.is_ok() && r2.is_ok() {
            logger.log(&format!(
                "OK: rolled back to previous runtime payload {}",
                prev.display()
            ));
        } else {
            logger.log(&format!(
                "FAIL: runtime payload rollback failed: x64={r1:?} x86={r2:?}"
            ));
        }
    }

    fn log_runtime_payload_hashes(
        install_root: &Path,
        payload_root: &Path,
        dll_x64: &Path,
        dll_x86: &Path,
        logger: &mut Logger,
    ) -> Result<(), String> {
        let rel = relative_to_root(install_root, payload_root);
        let h64 = sha256_file_hex(dll_x64)?;
        let h86 = sha256_file_hex(dll_x86)?;
        logger.log(&format!(
            "OK: verified runtime payload={rel} x64={} sha256={h64} x86={} sha256={h86}",
            dll_x64.display(),
            dll_x86.display()
        ));
        Ok(())
    }

    fn run_health_check(
        install_root: &Path,
        payload_root: &Path,
        dll_x64: &Path,
        dll_x86: &Path,
        machine_registration: bool,
        language_list_expected: bool,
        logger: &mut Logger,
    ) -> bool {
        let version = fs::read_to_string(install_root.join("VERSION"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(missing)".to_string());
        let com_status = check_com_registration(dll_x64, dll_x86, machine_registration);
        let profile_status = check_profile_registry(machine_registration);
        let language_status = if !language_list_expected {
            "skipped".to_string()
        } else if is_tip_in_user_language_list(SIMPLIFIED_CHINESE_TIP) {
            "ok".to_string()
        } else {
            "missing".to_string()
        };
        let probe = probe_engine_health(install_root);

        let mut reasons = Vec::new();
        if com_status != "ok" {
            reasons.push(format!("com_{com_status}"));
        }
        if profile_status != "ok" {
            reasons.push(format!("profile_{profile_status}"));
        }
        if language_status != "ok" && language_status != "skipped" {
            reasons.push(format!("language_{language_status}"));
        }
        if probe.status != "ok" {
            reasons.push(format!("engine_probe_{}", probe.status));
        }

        let healthy = reasons.is_empty();
        let status = if healthy { "ok" } else { "failed" };
        let reason = if healthy {
            "none".to_string()
        } else {
            reasons.join("+")
        };
        logger.log(&format!(
            "event=install_health_check status={} version={} machine={} install_root={} payload_root={} com={} profile={} language_list={} engine_probe={} engine_direct_probe={} engine_pipe_probe={} probe_exit={} probe_reason={} reason={}",
            format_health_value(status),
            format_health_value(&version),
            if machine_registration { 1 } else { 0 },
            format_health_value(&install_root.display().to_string()),
            format_health_value(&payload_root.display().to_string()),
            format_health_value(&com_status),
            format_health_value(&profile_status),
            format_health_value(&language_status),
            format_health_value(&probe.status),
            format_health_value(&probe.direct),
            format_health_value(&probe.pipe),
            format_health_value(&probe.exit_code),
            format_health_value(&probe.reason),
            format_health_value(&reason),
        ));
        healthy
    }

    fn cmd_stop_processes(args: &[String]) -> Result<i32, String> {
        let mut set_maintenance = false;
        let mut clear_maintenance = false;
        for arg in args {
            match arg.as_str() {
                "--set-maintenance" => set_maintenance = true,
                "--clear-maintenance" => clear_maintenance = true,
                _ => {}
            }
        }
        if set_maintenance {
            write_install_maintenance_flag(true);
        }
        let stopped = stop_running_helpers();
        if clear_maintenance {
            write_install_maintenance_flag(false);
        }
        println!("OK: stopped {stopped} helper process(es)");
        Ok(0)
    }

    fn cmd_install_machine(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut source: Option<PathBuf> = None;
        let mut verify_core_only = false;
        let mut skip_verify = false;
        let mut skip_acl = false;
        let mut skip_registration = false;
        let mut skip_runtime_cleanup = false;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                "--source" => {
                    source = Some(PathBuf::from(take_flag_value(args, &mut i, "--source")?))
                }
                "--verify-core-only" => verify_core_only = true,
                "--skip-verify" => skip_verify = true,
                "--skip-acl" => skip_acl = true,
                "--skip-registration" => skip_registration = true,
                "--skip-runtime-cleanup" => skip_runtime_cleanup = true,
                other => return Err(format!("unknown install-machine flag: {other}")),
            }
            i += 1;
        }

        let install_root = normalize_lexical(&root.unwrap_or_else(machine_install_root));
        assert_trusted_install_root(&install_root)?;
        let mut logger = Logger::new(Some(install_root.join("install_machine.log")), false);
        logger.log("event=install_session_start");
        logger.log(&format!(
            "BEGIN: srf_ime_setup install-machine root={} verify_core_only={} skip_verify={}",
            install_root.display(),
            verify_core_only,
            skip_verify
        ));

        let result = (|| -> Result<(), String> {
            if let Some(ref src) = source {
                copy_package_to_install_root(src, &install_root, &mut logger)?;
            }

            let payload_root = resolve_runtime_payload_root(&install_root, false)?
                .ok_or_else(|| "missing runtime payload".to_string())?;
            let x64_root = resolve_runtime_payload_arch_root(&install_root, "x64", false)?
                .ok_or_else(|| "missing x64 runtime payload".to_string())?;
            let x86_root = resolve_runtime_payload_arch_root(&install_root, "x86", false)?
                .ok_or_else(|| "missing x86 runtime payload".to_string())?;
            let dll_x64 = x64_root.join("srf_tsf_tip.dll");
            let dll_x86 = x86_root.join("srf_tsf_tip.dll");
            if !dll_x64.is_file() {
                return Err(format!("Missing required file: {}", dll_x64.display()));
            }
            if !dll_x86.is_file() {
                return Err(format!("Missing required file: {}", dll_x86.display()));
            }

            if skip_verify {
                logger.log("SKIP: package integrity verification suppressed by caller");
            } else {
                let core = if verify_core_only {
                    Some(core_manifest_files(&install_root, &dll_x64, &dll_x86))
                } else {
                    None
                };
                let mut stop_fn = || {
                    let _ = stop_running_helpers();
                };
                verify_package_hash_manifest(
                    &install_root,
                    core.as_deref(),
                    &mut stop_fn,
                    &mut |m| logger.log(m),
                )?;
            }

            if !skip_acl {
                set_install_directory_access(&install_root, true)?;
                logger.log("OK: verified machine install directory ACL");
            }

            let previous_payload = resolve_successful_runtime_payload_root(&install_root);
            log_runtime_payload_hashes(
                &install_root,
                &payload_root,
                &dll_x64,
                &dll_x86,
                &mut logger,
            )?;

            if !skip_registration {
                let reg_log = install_root.join("tip_registration.log");
                let reg_res = (|| -> Result<(), String> {
                    run_regsvr32(
                        "x64",
                        &dll_x64,
                        RegsvrMode::Register { machine: true },
                        Some(&reg_log),
                    )?;
                    run_regsvr32(
                        "x86",
                        &dll_x86,
                        RegsvrMode::Register { machine: true },
                        Some(&reg_log),
                    )?;
                    Ok(())
                })();
                if let Err(err) = reg_res {
                    let msg = format!(
                        "TIP registration failed. Detail log: {}. {err}",
                        reg_log.display()
                    );
                    logger.log(&format!("FAIL: {msg}"));
                    try_rollback_runtime(
                        &install_root,
                        previous_payload.as_deref(),
                        true,
                        &reg_log,
                        &mut logger,
                    );
                    return Err(msg);
                }
                logger.log(&format!(
                    "OK: completed TIP registration; detail log: {}",
                    reg_log.display()
                ));
            }

            write_runtime_payload_markers(&install_root, &payload_root)?;

            if skip_runtime_cleanup {
                logger.log("SKIP: runtime payload cleanup suppressed by caller");
            } else {
                cleanup_old_runtime_payloads(&install_root, &payload_root, &mut |m| logger.log(m));
            }

            logger.log(&format!(
                "OK: machine install complete for {}",
                install_root.display()
            ));
            Ok(())
        })();

        if let Err(ref err) = result {
            logger.log(&format!("FAIL: {err}"));
            return Err(err.clone());
        }
        Ok(0)
    }

    fn cmd_install_user(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut source: Option<PathBuf> = None;
        let mut use_machine = false;
        let mut desktop_task = false;
        let mut verify_core_only = false;
        let mut skip_verify = false;
        let mut skip_acl = false;
        let mut skip_registration = false;
        let mut skip_language_list = false;
        let mut skip_tray = false;
        let mut skip_text_service_restart = false;
        let mut skip_health_check = false;
        let mut skip_runtime_cleanup = false;
        let mut report_setup_status = false;
        let mut status_file: Option<PathBuf> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                "--source" => {
                    source = Some(PathBuf::from(take_flag_value(args, &mut i, "--source")?))
                }
                "--use-machine-registration" => use_machine = true,
                "--desktop-task" => desktop_task = true,
                "--verify-core-only" => verify_core_only = true,
                "--skip-verify" => skip_verify = true,
                "--skip-acl" => skip_acl = true,
                "--skip-registration" => skip_registration = true,
                "--skip-language-list" => skip_language_list = true,
                "--skip-tray" => skip_tray = true,
                "--skip-text-service-restart" => skip_text_service_restart = true,
                "--skip-health-check" => skip_health_check = true,
                "--skip-runtime-cleanup" => skip_runtime_cleanup = true,
                "--report-setup-status" => report_setup_status = true,
                "--status-file" => {
                    status_file = Some(PathBuf::from(take_flag_value(
                        args,
                        &mut i,
                        "--status-file",
                    )?))
                }
                other => return Err(format!("unknown install-user flag: {other}")),
            }
            i += 1;
        }

        let install_root = match root {
            Some(p) => normalize_lexical(&p),
            None => default_user_install_root()?,
        };
        let use_machine = use_machine || is_machine_install_root(&install_root);

        if use_machine && !desktop_task && is_current_process_elevated() {
            let sid = find_desktop_user_sid().ok_or_else(|| {
                "No signed-in desktop user is available to enable the input method.".to_string()
            })?;
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let mut forwarded = vec![
                "install-user".to_string(),
                "--root".to_string(),
                format!("\"{}\"", install_root.display()),
                "--desktop-task".to_string(),
                "--use-machine-registration".to_string(),
            ];
            if verify_core_only {
                forwarded.push("--verify-core-only".to_string());
            }
            if skip_verify {
                forwarded.push("--skip-verify".to_string());
            }
            if skip_acl {
                forwarded.push("--skip-acl".to_string());
            }
            if skip_registration {
                forwarded.push("--skip-registration".to_string());
            }
            if skip_language_list {
                forwarded.push("--skip-language-list".to_string());
            }
            if skip_tray {
                forwarded.push("--skip-tray".to_string());
            }
            if skip_text_service_restart {
                forwarded.push("--skip-text-service-restart".to_string());
            }
            if skip_health_check {
                forwarded.push("--skip-health-check".to_string());
            }
            if skip_runtime_cleanup {
                forwarded.push("--skip-runtime-cleanup".to_string());
            }
            if report_setup_status {
                forwarded.push("--report-setup-status".to_string());
            }
            let code = invoke_desktop_user_task("KaixinInputSetup", &sid, &exe, &forwarded)?;
            if report_setup_status && code == 10 {
                return Ok(10);
            }
            if code != 0 {
                return Err(format!("Desktop user setup failed ({code})."));
            }
            return Ok(0);
        }

        let state_root = user_state_root()?;
        let log_path = state_root.join("install_user.log");
        let mut logger = Logger::new(Some(log_path), false);
        logger.log("event=install_session_start");
        logger.log(&format!(
            "BEGIN: srf_ime_setup install-user root={} use_machine={} verify_core_only={} skip_verify={}",
            install_root.display(),
            use_machine,
            verify_core_only,
            skip_verify
        ));

        let outcome = (|| -> Result<i32, String> {
            if let Some(ref src) = source {
                assert_trusted_install_root(&install_root)?;
                copy_package_to_install_root(src, &install_root, &mut logger)?;
            }

            let payload_root = resolve_runtime_payload_root(&install_root, false)?
                .ok_or_else(|| "missing runtime payload".to_string())?;
            let x64_root = resolve_runtime_payload_arch_root(&install_root, "x64", false)?
                .ok_or_else(|| "missing x64 runtime payload".to_string())?;
            let x86_root = resolve_runtime_payload_arch_root(&install_root, "x86", false)?
                .ok_or_else(|| "missing x86 runtime payload".to_string())?;
            let dll_x64 = x64_root.join("srf_tsf_tip.dll");
            let dll_x86 = x86_root.join("srf_tsf_tip.dll");
            if !dll_x64.is_file() {
                return Err(format!("Missing required file: {}", dll_x64.display()));
            }
            if !dll_x86.is_file() {
                return Err(format!("Missing required file: {}", dll_x86.display()));
            }

            if skip_verify {
                logger.log("SKIP: package integrity verification delegated to machine step");
            } else {
                let core = if verify_core_only {
                    Some(core_manifest_files(&install_root, &dll_x64, &dll_x86))
                } else {
                    None
                };
                let mut stop_fn = || {
                    let _ = stop_running_helpers();
                };
                verify_package_hash_manifest(
                    &install_root,
                    core.as_deref(),
                    &mut stop_fn,
                    &mut |m| logger.log(m),
                )?;
            }

            if !(skip_acl || use_machine) {
                set_install_directory_access(&install_root, false)?;
                logger.log("OK: verified user install directory ACL");
            }

            clear_transient_user_state(&mut |m| logger.log(m))?;
            logger.log("event=install_session_start");

            let config_path = state_root.join("kaixin.ini");
            migrate_user_config(&config_path, &install_root, &mut |m| logger.log(m))?;

            if skip_runtime_cleanup || use_machine {
                logger.log("SKIP: runtime payload cleanup delegated or deferred");
            } else {
                cleanup_old_runtime_payloads(&install_root, &payload_root, &mut |m| logger.log(m));
            }

            let previous_payload = if use_machine {
                None
            } else {
                resolve_successful_runtime_payload_root(&install_root)
            };
            log_runtime_payload_hashes(
                &install_root,
                &payload_root,
                &dll_x64,
                &dll_x86,
                &mut logger,
            )?;

            if !skip_registration {
                ensure_ctfmon_running(&mut |m| logger.log(m));
                let reg_log = state_root.join("tip_registration_user.log");
                let reg_res = if use_machine {
                    let com_status = check_com_registration(&dll_x64, &dll_x86, true);
                    if com_status != "ok" {
                        Err(format!(
                            "Machine COM registration needs repair: {com_status}"
                        ))
                    } else {
                        remove_user_com_overrides(&mut |m| logger.log(m));
                        run_regsvr32("x64", &dll_x64, RegsvrMode::EnableProfile, Some(&reg_log))
                    }
                } else {
                    (|| -> Result<(), String> {
                        run_regsvr32(
                            "x64",
                            &dll_x64,
                            RegsvrMode::Register { machine: false },
                            Some(&reg_log),
                        )?;
                        run_regsvr32(
                            "x86",
                            &dll_x86,
                            RegsvrMode::Register { machine: false },
                            Some(&reg_log),
                        )?;
                        Ok(())
                    })()
                };
                if let Err(err) = reg_res {
                    let msg = format!(
                        "Current-user TIP setup failed. Detail log: {}. {err}",
                        reg_log.display()
                    );
                    if !use_machine {
                        try_rollback_runtime(
                            &install_root,
                            previous_payload.as_deref(),
                            false,
                            &reg_log,
                            &mut logger,
                        );
                    }
                    return Err(msg);
                }
                if !use_machine {
                    write_runtime_payload_markers(&install_root, &payload_root)?;
                }
                logger.log(&format!(
                    "OK: completed current-user TIP setup; detail log: {}",
                    reg_log.display()
                ));
            }

            if !skip_language_list {
                if let Err(err) =
                    add_tip_to_current_user_language_list(SIMPLIFIED_CHINESE_TIP, &mut |m| {
                        logger.log(m)
                    })
                {
                    logger.log(&format!(
                        "WARN: current-user language list setup failed; continuing install: {err}"
                    ));
                }
            }

            update_text_input_services(!skip_text_service_restart, &mut |m| logger.log(m));

            if !skip_tray {
                write_tray_run_entry(&install_root, &mut |m| logger.log(m))?;
                start_tray_helper(&install_root, &mut |m| logger.log(m));
            }

            let healthy = if skip_health_check {
                logger.log("SKIP: current-user install health check suppressed by caller");
                true
            } else {
                run_health_check(
                    &install_root,
                    &payload_root,
                    &dll_x64,
                    &dll_x86,
                    use_machine,
                    !skip_language_list,
                    &mut logger,
                )
            };

            logger.log(&format!(
                "OK: current-user setup complete for {}",
                install_root.display()
            ));
            if report_setup_status && !skip_health_check && !healthy {
                Ok(10)
            } else {
                Ok(0)
            }
        })();

        let final_code = match &outcome {
            Ok(code) => *code,
            Err(err) => {
                logger.log(&format!("FAIL: {err}"));
                1
            }
        };
        if let Some(ref sf) = status_file {
            let _ = fs::write(sf, final_code.to_string());
        }
        outcome
    }

    fn cmd_uninstall_user(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut from_machine = false;
        let mut desktop_task = false;
        let mut skip_registration = false;
        let mut skip_language_list = false;
        let mut skip_runtime_cleanup = false;
        let mut status_file: Option<PathBuf> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                "--from-machine-uninstaller" => from_machine = true,
                "--desktop-task" => desktop_task = true,
                "--skip-registration" => skip_registration = true,
                "--skip-language-list" => skip_language_list = true,
                "--skip-runtime-cleanup" => skip_runtime_cleanup = true,
                "--status-file" => {
                    status_file = Some(PathBuf::from(take_flag_value(
                        args,
                        &mut i,
                        "--status-file",
                    )?))
                }
                other => return Err(format!("unknown uninstall-user flag: {other}")),
            }
            i += 1;
        }

        let install_root = normalize_lexical(&root.unwrap_or_else(exe_dir));

        if from_machine && !desktop_task {
            if let (Some(desktop_sid), Some(current_sid)) =
                (find_desktop_user_sid(), current_user_sid_string())
            {
                if !desktop_sid.eq_ignore_ascii_case(&current_sid) {
                    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
                    let mut forwarded = vec![
                        "uninstall-user".to_string(),
                        "--root".to_string(),
                        format!("\"{}\"", install_root.display()),
                        "--desktop-task".to_string(),
                        "--skip-runtime-cleanup".to_string(),
                    ];
                    if skip_language_list {
                        forwarded.push("--skip-language-list".to_string());
                    }
                    if skip_registration {
                        forwarded.push("--skip-registration".to_string());
                    }
                    let code = invoke_desktop_user_task(
                        "KaixinInputUninstall",
                        &desktop_sid,
                        &exe,
                        &forwarded,
                    )?;
                    if code != 0 {
                        return Err(format!("Desktop user uninstall cleanup failed ({code})."));
                    }
                    return Ok(0);
                }
            }
        }

        let mut logger = Logger::new(None, true);
        let mut failures: Vec<String> = Vec::new();

        remove_tray_run_entries(&mut |m| logger.log(m));

        if !skip_language_list {
            if let Err(err) = restore_microsoft_pinyin(&mut |m| logger.log(m)) {
                failures.push(err);
            }
        }

        let payload_root = resolve_runtime_payload_root(&install_root, true)?;
        let x64_root = resolve_runtime_payload_arch_root(&install_root, "x64", true)?;
        let x86_root = resolve_runtime_payload_arch_root(&install_root, "x86", true)?;
        let dll_x64 = x64_root
            .map(|d| d.join("srf_tsf_tip.dll"))
            .or_else(|| payload_root.map(|d| d.join("srf_tsf_tip.dll")))
            .unwrap_or_else(|| install_root.join("srf_tsf_tip.dll"));
        let dll_x86 = x86_root.map(|d| d.join("srf_tsf_tip.dll"));

        if !skip_registration && dll_x64.is_file() {
            if let Err(err) = run_regsvr32(
                "x64",
                &dll_x64,
                RegsvrMode::Unregister { machine: false },
                None,
            ) {
                failures.push(err);
            }
            if let Some(ref x86) = dll_x86 {
                if x86.is_file() {
                    if let Err(err) =
                        run_regsvr32("x86", x86, RegsvrMode::Unregister { machine: false }, None)
                    {
                        failures.push(err);
                    }
                }
            }
        } else {
            logger.log("SKIP: TIP unregistration not requested or DLL missing");
        }

        let _ = stop_running_helpers();
        if let Err(err) = remove_kaixin_user_data(&mut |m| logger.log(m)) {
            failures.push(err);
        }

        if !(skip_runtime_cleanup || from_machine) {
            let runtime_root = install_root.join(RUNTIME_PAYLOAD_ROOT_NAME);
            if runtime_root.exists()
                && !schedule_pending_delete_tree(&runtime_root, &mut |m| logger.log(m))
            {
                let marker = install_root.join(RUNTIME_PAYLOAD_MANIFEST_NAME);
                let _ = register_hkcu_runonce_cleanup(&runtime_root, Some(&marker), &mut |m| {
                    logger.log(m)
                });
            }
        }

        let final_code = if failures.is_empty() { 0 } else { 1 };
        if let Some(ref sf) = status_file {
            let _ = fs::write(sf, final_code.to_string());
        }
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        Ok(0)
    }

    fn cmd_uninstall_machine(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut skip_file_removal = false;
        let mut skip_registration = false;
        let mut skip_uninstall_entry = false;
        let mut skip_current_user_cleanup = false;
        let mut skip_language_list = false;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                "--skip-file-removal" => skip_file_removal = true,
                "--skip-registration" => skip_registration = true,
                "--skip-uninstall-entry" => skip_uninstall_entry = true,
                "--skip-current-user-cleanup" => skip_current_user_cleanup = true,
                "--skip-language-list" => skip_language_list = true,
                other => return Err(format!("unknown uninstall-machine flag: {other}")),
            }
            i += 1;
        }

        let install_root = normalize_lexical(&root.unwrap_or_else(exe_dir));
        let mut logger = Logger::new(None, true);

        if !skip_current_user_cleanup {
            let mut u_args = vec![
                "--root".to_string(),
                install_root.display().to_string(),
                "--from-machine-uninstaller".to_string(),
                "--skip-registration".to_string(),
                "--skip-runtime-cleanup".to_string(),
            ];
            if skip_language_list {
                u_args.push("--skip-language-list".to_string());
            }
            let _ = cmd_uninstall_user(&u_args)?;
        }

        let payload_root = resolve_runtime_payload_root(&install_root, true)?;
        let x64_root = resolve_runtime_payload_arch_root(&install_root, "x64", true)?;
        let x86_root = resolve_runtime_payload_arch_root(&install_root, "x86", true)?;
        let dll_x64 = x64_root
            .map(|d| d.join("srf_tsf_tip.dll"))
            .or_else(|| payload_root.map(|d| d.join("srf_tsf_tip.dll")))
            .unwrap_or_else(|| install_root.join("srf_tsf_tip.dll"));
        let dll_x86 = x86_root.map(|d| d.join("srf_tsf_tip.dll"));

        if !skip_registration && dll_x64.is_file() {
            if let Err(err) = run_regsvr32(
                "x64",
                &dll_x64,
                RegsvrMode::Unregister { machine: true },
                None,
            ) {
                logger.log(&format!("WARN: x64 TIP unregistration failed: {err}"));
            }
            if let Some(ref x86) = dll_x86 {
                if x86.is_file() {
                    if let Err(err) =
                        run_regsvr32("x86", x86, RegsvrMode::Unregister { machine: true }, None)
                    {
                        logger.log(&format!("WARN: x86 TIP unregistration failed: {err}"));
                    }
                }
            }
            remove_tip_from_loaded_user_registry(SIMPLIFIED_CHINESE_TIP, &mut |m| logger.log(m));
        }

        if !skip_uninstall_entry {
            remove_uninstall_registry_entry(true, &mut |m| logger.log(m));
        }

        if let Err(err) = remove_machine_state_root() {
            logger.log(&format!("WARN: could not remove machine state root: {err}"));
        }

        if !skip_file_removal {
            assert_trusted_install_root(&install_root)?;
            if !install_root.join("srf_ime_setup.exe").is_file() && !dll_x64.is_file() {
                return Err(format!(
                    "Refusing to remove {} because no install sentinel files were found.",
                    install_root.display()
                ));
            }
            schedule_self_deferred_dir_delete(&install_root);
            if !schedule_pending_delete_tree(&install_root, &mut |m| logger.log(m)) {
                let _ = register_hkcu_runonce_cleanup(&install_root, None, &mut |m| logger.log(m));
            }
        }

        logger.log(&format!(
            "OK: uninstall cleanup complete for {}",
            install_root.display()
        ));
        Ok(0)
    }

    fn cmd_repair(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                other => return Err(format!("unknown repair flag: {other}")),
            }
            i += 1;
        }
        let install_root = normalize_lexical(&root.unwrap_or_else(exe_dir));
        let machine = is_machine_install_root(&install_root);
        let mut user_args = vec!["--root".to_string(), install_root.display().to_string()];

        if machine {
            user_args.push("--use-machine-registration".to_string());
        }
        if machine && is_current_process_elevated() {
            println!("正在全量校验安装包文件并修复系统级 TSF 注册…");
            let m_args = vec!["--root".to_string(), install_root.display().to_string()];
            cmd_install_machine(&m_args)?;
            user_args.push("--skip-verify".to_string());
            user_args.push("--skip-runtime-cleanup".to_string());
        } else {
            println!("正在全量校验安装包文件并修复当前用户输入法配置…");
        }

        let code = cmd_install_user(&user_args)?;
        if code != 0 {
            return Err(format!("Repair failed with exit code {code}"));
        }
        println!("开心输入法修复完成。");
        Ok(0)
    }

    fn cmd_cleanup_runtime(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                other => return Err(format!("unknown cleanup-runtime flag: {other}")),
            }
            i += 1;
        }
        let install_root = normalize_lexical(&root.unwrap_or_else(exe_dir));
        let Some(payload_root) = resolve_runtime_payload_root(&install_root, true)? else {
            return Ok(0);
        };
        let mut logger = Logger::new(None, false);
        cleanup_old_runtime_payloads(&install_root, &payload_root, &mut |m| logger.log(m));
        Ok(0)
    }

    fn cmd_verify_manifest(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut core_files: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                "--core-file" => core_files.push(take_flag_value(args, &mut i, "--core-file")?),
                other => return Err(format!("unknown verify-manifest flag: {other}")),
            }
            i += 1;
        }
        let target = root.ok_or_else(|| "missing --root".to_string())?;
        let mut logger = Logger::new(None, true);
        let core_opt = if core_files.is_empty() {
            None
        } else {
            Some(core_files.as_slice())
        };
        let mut noop = || {};
        verify_package_hash_manifest(&target, core_opt, &mut noop, &mut |m| logger.log(m))?;
        Ok(0)
    }

    fn cmd_ensure_acl(args: &[String]) -> Result<i32, String> {
        let mut root: Option<PathBuf> = None;
        let mut machine = false;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--root" => root = Some(PathBuf::from(take_flag_value(args, &mut i, "--root")?)),
                "--machine" => machine = true,
                other => return Err(format!("unknown ensure-acl flag: {other}")),
            }
            i += 1;
        }
        let target = root.ok_or_else(|| "missing --root".to_string())?;
        set_install_directory_access(&target, machine)?;
        Ok(0)
    }

    pub fn run() -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            eprintln!(
                "Usage: srf_ime_setup <stop-processes|install-machine|install-user|uninstall-user|uninstall-machine|repair|cleanup-runtime|verify-manifest|ensure-acl> [options]"
            );
            return 2;
        }
        let sub = args[0].as_str();
        let rest = &args[1..];
        let res = match sub {
            "stop-processes" => cmd_stop_processes(rest),
            "install-machine" => cmd_install_machine(rest),
            "install-user" => cmd_install_user(rest),
            "uninstall-user" => cmd_uninstall_user(rest),
            "uninstall-machine" => cmd_uninstall_machine(rest),
            "repair" => cmd_repair(rest),
            "cleanup-runtime" => cmd_cleanup_runtime(rest),
            "verify-manifest" => cmd_verify_manifest(rest),
            "ensure-acl" => cmd_ensure_acl(rest),
            other => Err(format!("unknown subcommand: {other}")),
        };
        match res {
            Ok(code) => code,
            Err(err) => {
                eprintln!("ERROR: {err}");
                1
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    std::process::exit(cli::run());
}
