fn read_tray_visual_state() -> TrayVisualState {
    let sequence_before = read_state_qword(STATE_REG_VALUE_INPUT_SEQUENCE);
    let engine_health = pinyin_ime::engine_health::current();
    let mut state = TrayVisualState {
        ascii_mode: read_state_dword(STATE_REG_VALUE_INPUT_ASCII).map(|value| value != 0),
        mode_source: read_state_string(STATE_REG_VALUE_INPUT_MODE_SOURCE),
        full_shape: read_state_dword(STATE_REG_VALUE_FULL_SHAPE).map(|value| value != 0),
        chinese_punctuation: read_state_dword(STATE_REG_VALUE_CHINESE_PUNCTUATION)
            .map(|value| value != 0),
        owner_process_id: read_state_dword(STATE_REG_VALUE_INPUT_OWNER_PROCESS_ID),
        owner_thread_id: read_state_dword(STATE_REG_VALUE_INPUT_OWNER_THREAD_ID),
        owner_hwnd: read_state_qword(STATE_REG_VALUE_INPUT_OWNER_HWND),
        updated_tick: read_state_qword(STATE_REG_VALUE_INPUT_UPDATED_TICK),
        sequence: sequence_before,
        engine_state: TrayEngineState::from_registry(Some(engine_health.state)),
        engine_error_code: engine_health.error_code,
        last_recovery_reason: read_state_string(STATE_REG_VALUE_LAST_ENGINE_RECOVERY_REASON),
        last_recovery_time: read_state_string(STATE_REG_VALUE_LAST_ENGINE_RECOVERY_TIME),
    };
    let sequence_after = read_state_qword(STATE_REG_VALUE_INPUT_SEQUENCE);
    if sequence_before.is_none()
        || sequence_before != sequence_after
        || !input_snapshot_matches_foreground(&state)
    {
        state.ascii_mode = None;
        state.mode_source = None;
    }
    apply_pure_game_visual_override(&mut state, pure_game_mode_enabled());
    state
}

fn pure_game_mode_enabled() -> bool {
    read_state_dword(STATE_REG_VALUE_PURE_GAME_MODE).unwrap_or(0) != 0
}

fn apply_pure_game_visual_override(state: &mut TrayVisualState, enabled: bool) {
    if enabled {
        // The explicit user switch is authoritative even without an active
        // TSF host, or while an old foreground snapshot is being discarded.
        state.ascii_mode = Some(true);
        state.mode_source = Some("pure_game".into());
        state.full_shape = Some(false);
        state.chinese_punctuation = Some(false);
    } else if state.mode_source.as_deref() == Some("pure_game") {
        // Do not advertise a stale TSF snapshot after the user has unchecked
        // the switch. The next TSF publication supplies the restored mode.
        state.ascii_mode = None;
        state.mode_source = None;
        state.full_shape = None;
        state.chinese_punctuation = None;
    }
}

fn input_snapshot_matches_foreground(state: &TrayVisualState) -> bool {
    let (Some(owner_pid), Some(owner_hwnd), Some(updated_tick), Some(sequence)) = (
        state.owner_process_id,
        state.owner_hwnd,
        state.updated_tick,
        state.sequence,
    ) else {
        return false;
    };
    if owner_pid == 0 || owner_hwnd == 0 || sequence == 0 {
        return false;
    }
    // A tick from a previous boot cannot describe the current foreground
    // context. Long-lived but still focused game sessions remain valid.
    if updated_tick > unsafe { GetTickCount64() } {
        return false;
    }
    let owner_hwnd = owner_hwnd as HWND;
    if unsafe { IsWindow(owner_hwnd) } == 0 {
        return false;
    }
    let foreground = unsafe { GetForegroundWindow() };
    if foreground == 0 {
        return false;
    }
    let mut foreground_pid = 0u32;
    let mut published_hwnd_pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(foreground, &mut foreground_pid);
        GetWindowThreadProcessId(owner_hwnd, &mut published_hwnd_pid);
    }
    let foreground_is_tray = foreground_pid == unsafe { GetCurrentProcessId() };
    (foreground_pid == owner_pid || foreground_is_tray) && published_hwnd_pid == owner_pid
}

fn tray_tooltip_text(state: &TrayVisualState) -> String {
    if state.mode_source.as_deref() == Some("pure_game") {
        return format!("{TRAY_TIP}\n纯游戏模式：键盘完全直通\n输入法与工具快捷键已停用\n右键托盘取消勾选即可恢复");
    }
    let mode = match state.input_mode() {
        TrayInputMode::Chinese => "中文",
        TrayInputMode::English => "英文",
        TrayInputMode::Unknown => "状态未知",
    };
    let punctuation = match state.chinese_punctuation {
        Some(true) => "中文",
        Some(false) => "英文",
        None => "未知",
    };
    let shape = match state.full_shape {
        Some(true) => "全角",
        Some(false) => "半角",
        None => "未知",
    };
    let source = mode_source_label(state.mode_source.as_deref());
    let engine = match state.engine_error_code {
        Some(code) if state.engine_state == TrayEngineState::Failed => {
            format!("{}（故障码 {code}）", state.engine_state.label())
        }
        _ => state.engine_state.label().to_string(),
    };

    let mut lines = vec![
        format!("{TRAY_TIP}  {mode}"),
        format!("标点：{punctuation}  字符：{shape}"),
        format!("来源：{source}"),
        format!("引擎：{engine}"),
    ];
    if let Some(reason) = state.last_recovery_reason.as_deref() {
        let time = state
            .last_recovery_time
            .as_deref()
            .map(short_time_label)
            .unwrap_or("时间未知");
        lines.push(format!("历史恢复：{time} {}", shorten_chars(reason, 30)));
    }
    lines.join("\n")
}

fn mode_source_label(source: Option<&str>) -> &'static str {
    match source {
        Some("pure_game") => "纯游戏模式 · 完全直通",
        Some("manual") => "手动英文",
        Some("game") => "游戏键盘直通",
        Some("game_chat") => "游戏中文聊天",
        Some("fullscreen") => "全屏兼容",
        Some("app") => "应用配置",
        Some("privacy") => "隐私保护",
        Some("fallback") => "兼容回退",
        Some("recovery") => "手动恢复中文",
        Some("global") => "全局英文",
        Some("english") => "英文",
        Some("chinese") => "中文",
        _ => "未知",
    }
}

fn short_time_label(value: &str) -> &str {
    value
        .rsplit_once(' ')
        .map(|(_, tail)| tail)
        .unwrap_or(value)
}

fn shorten_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn read_ascii_mode() -> Option<bool> {
    // AsciiMode=1 表示英文；0/缺失表示中文。
    read_state_dword(STATE_REG_VALUE_ASCII).map(|value| value != 0)
}

fn install_maintenance_active() -> bool {
    if read_state_dword(STATE_REG_VALUE_INSTALL_MAINTENANCE).unwrap_or(0) == 0 {
        return false;
    }
    let Some(started_at) = read_state_dword(STATE_REG_VALUE_INSTALL_MAINTENANCE_TICK) else {
        return false;
    };
    unsafe { GetTickCount() }.wrapping_sub(started_at) <= INSTALL_MAINTENANCE_MAX_AGE_MS
}

fn read_state_dword(value_name: &str) -> Option<u32> {
    let mut value: u32 = 0;
    let mut cb: u32 = size_of::<u32>() as u32;
    let subkey = wide(STATE_REG_PATH);
    let name = wide(value_name);
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut cb,
        )
    };
    // ERROR_SUCCESS == 0
    (rc == 0).then_some(value)
}

fn read_state_qword(value_name: &str) -> Option<u64> {
    let mut value = 0u64;
    let mut cb = size_of::<u64>() as u32;
    let subkey = wide(STATE_REG_PATH);
    let name = wide(value_name);
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_QWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u64).cast(),
            &mut cb,
        )
    };
    (rc == 0).then_some(value)
}

fn read_state_string(value_name: &str) -> Option<String> {
    let subkey = wide(STATE_REG_PATH);
    let name = wide(value_name);
    let mut bytes = 0u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if rc != 0 || bytes <= 2 {
        return None;
    }

    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if rc != 0 {
        return None;
    }
    let len = buffer
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..len]).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod pure_game_tests {
    use super::*;

    fn state() -> TrayVisualState {
        TrayVisualState {
            ascii_mode: Some(false),
            mode_source: Some("chinese".into()),
            full_shape: Some(true),
            chinese_punctuation: Some(true),
            owner_process_id: None,
            owner_thread_id: None,
            owner_hwnd: None,
            updated_tick: None,
            sequence: None,
            engine_state: TrayEngineState::Ready,
            engine_error_code: None,
            last_recovery_reason: None,
            last_recovery_time: None,
        }
    }

    #[test]
    fn explicit_pure_game_switch_overrides_even_a_missing_host_snapshot() {
        let mut live = state();
        live.ascii_mode = None;
        live.mode_source = None;
        apply_pure_game_visual_override(&mut live, true);
        assert!(live.input_mode() == TrayInputMode::English);
        assert_eq!(live.mode_source.as_deref(), Some("pure_game"));
        assert_eq!(live.full_shape, Some(false));
        assert_eq!(live.chinese_punctuation, Some(false));
        let tooltip = tray_tooltip_text(&live);
        assert!(tooltip.contains("完全直通") && tooltip.contains("快捷键已停用"));
        assert!(tooltip.contains("取消勾选"));
    }

    #[test]
    fn disabling_override_preserves_normal_modes_and_rejects_stale_game_status() {
        let normal = state();
        let mut restored = normal.clone();
        apply_pure_game_visual_override(&mut restored, false);
        assert!(restored == normal);
        apply_pure_game_visual_override(&mut restored, true);
        apply_pure_game_visual_override(&mut restored, false);
        assert!(restored.mode_source.is_none() && restored.ascii_mode.is_none());
        assert!(!tray_tooltip_text(&restored).contains("快捷键已停用"));
    }
}
