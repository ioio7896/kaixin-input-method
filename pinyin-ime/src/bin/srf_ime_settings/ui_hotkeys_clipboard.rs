use super::*;

pub(super) fn hotkeys_ui(ui: &mut egui::Ui, model: &mut SettingsModel) {
    let translate_available = translation_available();
    section_panel(ui, "快捷键", |ui| {
        setting_combo_row(
            ui,
            "中英切换",
            "切换中文输入与英文直输模式。",
            cn_en_hotkey_label(&model.cn_en_hotkey),
            "cn_en_hotkey",
            |ui| {
                selectable_string(
                    ui,
                    &mut model.cn_en_hotkey,
                    "both",
                    "Ctrl+Shift / Ctrl+Space",
                );
                selectable_string(ui, &mut model.cn_en_hotkey, "shift", "Ctrl+Shift");
                selectable_string(ui, &mut model.cn_en_hotkey, "ctrl_space", "Ctrl+Space");
                selectable_string(ui, &mut model.cn_en_hotkey, "none", "关闭");
            },
        );
        setting_toggle(
            ui,
            "全半角切换",
            "使用 Shift+Space 在全角和半角之间切换；默认全角开启时会自动启用。",
            &mut model.full_shape_hotkey,
        );
        setting_toggle(
            ui,
            "标点切换",
            "使用 Ctrl+. 在中文标点和英文标点之间切换。",
            &mut model.punct_hotkey,
        );
        setting_toggle(
            ui,
            "模糊音切换",
            "使用 Ctrl+Shift+F 快速开关模糊音。",
            &mut model.fuzzy_hotkey,
        );
        setting_toggle(
            ui,
            "双拼切换",
            "使用 Ctrl+Shift+D 快速开关双拼。",
            &mut model.double_pinyin_hotkey,
        );
        setting_toggle(
            ui,
            "轻按 Shift 切换英文",
            "单击 Shift 临时进入英文直输。",
            &mut model.shift_tap_hotkey,
        );
        setting_toggle(
            ui,
            "默认繁体输出",
            "开启后候选输出为繁体；也可使用下方快捷键临时切换。",
            &mut model.traditional_output,
        );
        setting_row(
            ui,
            "简繁切换",
            "在当前输入法会话中切换简体/繁体输出。",
            |ui| {
                hotkey_combo(
                    ui,
                    "",
                    "traditional_hotkey",
                    &mut model.traditional_hotkey,
                    "F",
                );
            },
        );
        setting_row(
            ui,
            "游戏兼容模式",
            "手动切换游戏兼容策略，适合临时进入全屏游戏前使用。",
            |ui| {
                hotkey_combo(ui, "", "game_mode_hotkey", &mut model.game_mode_hotkey, "G");
            },
        );
        setting_row(
            ui,
            "临时强制英文",
            "手动切换临时英文直输。",
            |ui| {
                hotkey_combo(
                    ui,
                    "",
                    "temporary_ascii_hotkey",
                    &mut model.temporary_ascii_hotkey,
                    "Space",
                );
            },
        );
        setting_toggle(
            ui,
            "候选数字直选",
            "按数字键直接提交对应候选。",
            &mut model.candidate_number_select,
        );
        setting_toggle(
            ui,
            "翻页 - / =",
            "使用 - 和 = 翻页。",
            &mut model.page_minus_equal,
        );
        setting_toggle(
            ui,
            "翻页 , / .",
            "使用逗号和句号翻页。",
            &mut model.page_comma_period,
        );
        setting_toggle(
            ui,
            "翻页 PgUp / PgDn",
            "使用 PageUp 和 PageDown 翻页。",
            &mut model.page_pgup_pgdn,
        );
        setting_row(
            ui,
            "截图快捷键",
            "组合键固定为修饰键 + 字母。",
            |ui| {
                ui.vertical(|ui| {
                    fixed_letter_hotkey_combo(
                        ui,
                        "",
                        "screenshot_hotkey_letter",
                        &mut model.screenshot_hotkey,
                        'A',
                    );
                    hotkey_registration_status_label(ui, "registered");
                });
            },
        );
        setting_row(ui, "设置页快捷键", "全局打开设置页。", |ui| {
            hotkey_combo(
                ui,
                "",
                "settings_hotkey",
                &mut model.settings_hotkey,
                "Comma",
            );
        });
        setting_row(
            ui,
            "手写查字快捷键",
            "全局打开手写查字工具。",
            |ui| {
                hotkey_combo(ui, "", "handwrite_hotkey", &mut model.handwrite_hotkey, "H");
            },
        );
        setting_row(ui, "OCR 快捷键", "全局打开 OCR 工具。", |ui| {
            ui.vertical(|ui| {
                hotkey_combo(ui, "", "ocr_hotkey", &mut model.ocr_hotkey, "O");
                hotkey_registration_status_label(ui, "ocr_registered");
            });
        });
        setting_row(
            ui,
            "截图翻译快捷键",
            "截图识别后自动送入中英翻译。",
            |ui| {
                ui.vertical(|ui| {
                    hotkey_combo(
                        ui,
                        "",
                        "ocr_translate_hotkey",
                        &mut model.ocr_translate_hotkey,
                        "Y",
                    );
                    hotkey_registration_status_label(ui, "ocr_translate_registered");
                });
            },
        );
        setting_row(
            ui,
            "中英翻译快捷键",
            "全局打开中英翻译浮窗。",
            |ui| {
                ui.vertical(|ui| {
                    hotkey_combo(ui, "", "translate_hotkey", &mut model.translate_hotkey, "T");
                    hotkey_registration_status_label(ui, "translate_registered");
                    if !translate_available {
                        status_badge(ui, StatusTone::Warning, "模型未安装");
                    }
                });
            },
        );
        let palette = fluent_palette(ui);
        ui.label(
            RichText::new("快捷键冲突会以橙色提示。")
                .small()
                .color(palette.muted),
        );
    });
}

pub(super) fn clipboard_settings_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "剪贴板快捷键", |ui| {
        setting_row(
            ui,
            "剪贴板快捷键",
            "打开独立剪贴板管理器；组合键固定为修饰键 + 字母。",
            |ui| {
                fixed_letter_hotkey_combo(
                    ui,
                    "",
                    "clipboard_hotkey_letter",
                    &mut app.model.clipboard_hotkey,
                    'V',
                );
            },
        );
    });

    section_panel(ui, "历史记录", |ui| {
        setting_toggle(
            ui,
            "后台保存剪贴板历史",
            "关闭后不再记录系统剪贴板文本；已保存内容仍可在下方清空。",
            &mut app.model.clipboard_background_enabled,
        );
        setting_slider_usize(
            ui,
            "历史记录上限",
            "最多保留的普通剪贴板条数；设为 0 表示不保留普通历史。",
            &mut app.model.clipboard_max_history_items,
            0..=300,
        );
        setting_slider_usize(
            ui,
            "置顶记录上限",
            "最多保留的置顶剪贴板条数；设为 0 表示不保留置顶记录。",
            &mut app.model.clipboard_max_pinned_items,
            0..=100,
        );
        setting_slider_usize(
            ui,
            "单条文本上限",
            "单条文本最多保存的 UTF-16 单元数，过长内容会被截断。",
            &mut app.model.clipboard_max_text_utf16_units,
            20..=20_000,
        );
        setting_toggle(
            ui,
            "候选栏显示剪贴板片段",
            "关闭后仍可使用剪贴板候选，但候选栏不显示复制文本预览。",
            &mut app.model.clipboard_candidate_preview_enabled,
        );
        setting_toggle(
            ui,
            "记录来源应用",
            "开启后会在剪贴板历史中保存复制来源进程路径。",
            &mut app.model.clipboard_record_source_app,
        );
        setting_slider_usize(
            ui,
            "自动清理天数",
            "用于剪贴板管理器的自动清理参考；0 表示不按天数清理。",
            &mut app.model.clipboard_max_age_days,
            0..=3650,
        );
        setting_toggle(
            ui,
            "置顶也按天数清理",
            "开启后，按天数清理时也会处理置顶记录；清理天数为 0 时不按天数清理。",
            &mut app.model.clipboard_pinned_respects_max_age,
        );
    });

    let clipboard_path = pinyin_ime::clipboard_store::store_path();
    let clipboard_storage_status = clipboard_storage_status_text(&clipboard_path);
    section_panel(ui, "存储与维护", |ui| {
        setting_row(ui, "加密状态", &clipboard_storage_status, |_| {});
        data_location_row(
            ui,
            "历史数据库",
            "剪贴板历史仅在后台保存开启时写入本机数据库。",
            &clipboard_path,
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    app.open_data_location(clipboard_path.clone());
                }
            },
        );
        inline_notice(
            ui,
            StatusTone::Danger,
            "清空会删除本机已保存的剪贴板历史和置顶项。",
        );
        ui.add_space(6.0);
        if danger_button(ui, "清空历史").clicked() {
            app.clear_clipboard();
        }
    });

    section_panel(ui, "敏感应用", |ui| {
        ui.label("以下进程名单会以明文保存到 config.ini；匹配后不解析剪贴板历史候选。");
        privacy_process_list_row(
            ui,
            "永不剪贴板",
            "匹配这些进程时，不解析剪贴板历史候选。",
            &mut app.model.privacy_never_clipboard_processes,
        );
    });
}

struct HotkeyRegistrationStatus {
    enabled: bool,
    hotkey: Option<String>,
    reason: Option<String>,
    error: Option<String>,
}

fn hotkey_registration_status_label(ui: &mut egui::Ui, status_key: &str) {
    let (text, tone) = match read_hotkey_registration_status(status_key) {
        Some(status) if status.enabled => {
            let hotkey = status.hotkey.unwrap_or_else(|| "未知快捷键".to_string());
            (format!("已生效 {hotkey}"), StatusTone::Success)
        }
        Some(status) if status.reason.as_deref() == Some("disabled") => {
            ("已关闭".to_string(), StatusTone::Info)
        }
        Some(status) => {
            let detail = status
                .error
                .or(status.reason)
                .unwrap_or_else(|| "注册失败".to_string());
            (format!("注册失败 {detail}"), StatusTone::Warning)
        }
        None => ("等待托盘注册".to_string(), StatusTone::Info),
    };
    status_badge(ui, tone, &text);
}

fn read_hotkey_registration_status(status_key: &str) -> Option<HotkeyRegistrationStatus> {
    let path = app_paths::local_data_dir()?.join("screenshot_hotkey_status.txt");
    let body = fs::read_to_string(path).ok()?;
    parse_hotkey_registration_status(&body, status_key)
}

fn parse_hotkey_registration_status(
    body: &str,
    status_key: &str,
) -> Option<HotkeyRegistrationStatus> {
    let tokens = body.split_whitespace().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < tokens.len() {
        let Some((key, value)) = tokens[idx].split_once('=') else {
            idx += 1;
            continue;
        };
        if key != status_key {
            idx += 1;
            continue;
        }

        let mut status = HotkeyRegistrationStatus {
            enabled: value == "1",
            hotkey: None,
            reason: None,
            error: None,
        };
        idx += 1;
        while idx < tokens.len() {
            let Some((token_key, token_value)) = tokens[idx].split_once('=') else {
                idx += 1;
                continue;
            };
            if is_hotkey_status_start_token(token_key) {
                break;
            }
            match token_key {
                "hotkey" => status.hotkey = Some(token_value.to_string()),
                "reason" => status.reason = Some(token_value.to_string()),
                "error" => status.error = Some(token_value.to_string()),
                _ => {}
            }
            idx += 1;
        }
        return Some(status);
    }
    None
}

fn is_hotkey_status_start_token(key: &str) -> bool {
    key == "registered" || key.ends_with("_registered")
}
