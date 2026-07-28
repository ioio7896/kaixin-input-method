//! Windows：区分主键盘数字行与小键盘（egui 将二者都映射为 `Key::Num1`…）。
//! 在 `raw_input_hook` 里调整事件：小键盘去掉 `Key` 保留 `Text`；有候选时主键盘选词并去掉对应 `Text`。

use egui::{Event, RawInput};

fn key_to_digit_1_9(key: &egui::Key) -> Option<u8> {
    match key {
        egui::Key::Num1 => Some(1),
        egui::Key::Num2 => Some(2),
        egui::Key::Num3 => Some(3),
        egui::Key::Num4 => Some(4),
        egui::Key::Num5 => Some(5),
        egui::Key::Num6 => Some(6),
        egui::Key::Num7 => Some(7),
        egui::Key::Num8 => Some(8),
        egui::Key::Num9 => Some(9),
        _ => None,
    }
}

/// 当前帧物理上更像小键盘数字还是主键盘数字（`GetAsyncKeyState`）。
fn digit_key_is_numpad(d: u8) -> bool {
    debug_assert!((1..=9).contains(&d));
    let d = i32::from(d);
    // VK '1'..'9' = 0x31..0x39；VK NUMPAD1..9 = 0x61..0x69
    let vk_main = 0x30 + d;
    let vk_numpad = 0x60 + d;
    unsafe {
        let n = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk_numpad) as u16;
        let m = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk_main) as u16;
        let n_down = n & 0x8000 != 0;
        let m_down = m & 0x8000 != 0;
        if n_down && !m_down {
            return true;
        }
        if m_down && !n_down {
            return false;
        }
    }
    false
}

/// 过滤 `raw_input.events`，供 `eframe::App::raw_input_hook` 使用。
pub fn filter_raw_input(raw_input: &mut RawInput, has_candidates: bool) {
    // 1) 去掉小键盘数字的 Key 事件，避免被 IME 快捷键当成候选；数字仍由 `Event::Text` 进入拼音框。
    raw_input.events.retain(|e| {
        if let Event::Key {
            key, pressed: true, ..
        } = e
        {
            if let Some(d) = key_to_digit_1_9(key) {
                if digit_key_is_numpad(d) {
                    return false;
                }
            }
        }
        true
    });

    if !has_candidates {
        return;
    }

    // 2) 有候选时：主键盘行数字用于选词，去掉对应单位 `Text`，避免同时打进拼音框。
    let mut strip_text = [false; 10];
    for e in &raw_input.events {
        if let Event::Key {
            key, pressed: true, ..
        } = e
        {
            if let Some(d) = key_to_digit_1_9(key) {
                if !digit_key_is_numpad(d) {
                    strip_text[d as usize] = true;
                }
            }
        }
    }

    raw_input.events.retain(|e| {
        if let Event::Text(t) = e {
            if t.len() == 1 {
                if let Some(c) = t.chars().next().and_then(|ch| ch.to_digit(10)) {
                    if (1..=9).contains(&c) && strip_text[c as usize] {
                        return false;
                    }
                }
            }
        }
        true
    });
}
