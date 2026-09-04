use super::*;

pub(super) fn privacy_data_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let config_path = app.config_path.clone();
    let data_dir = app_paths::local_data_dir().unwrap_or_else(|| {
        config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });
    let user_dict_path = pinyin_ime::user_dict::default_user_dict_path();
    let log_path = tsf_trace_log_path();
    let encryption_status = user_dict_encryption_status_text(&user_dict_path);

    section_panel(ui, "信任中心", |ui| {
        data_location_row(
            ui,
            "本地数据目录",
            "配置、词库、剪贴板和日志所在目录。",
            &data_dir,
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    app.open_data_location(data_dir.clone());
                }
            },
        );
        setting_row(ui, "用户词库加密状态", &encryption_status, |ui| {
            if outline_button(ui, "导出隐私说明").clicked() {
                app.export_privacy_statement();
            }
        });
        data_location_row(
            ui,
            "配置文件",
            "输入法行为和界面设置。",
            &config_path,
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    app.open_data_location(config_path.clone());
                }
            },
        );
        data_location_row(
            ui,
            "用户词库",
            "本地学习词和上下文排序信号。",
            &user_dict_path,
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    app.open_data_location(user_dict_path.clone());
                }
            },
        );
        data_location_row(
            ui,
            "TSF 日志",
            "默认只记录脱敏诊断事件。",
            &log_path,
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    app.open_data_location(log_path.clone());
                }
            },
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "维护", |ui| {
        let palette = fluent_palette(ui);
        inline_notice(
            ui,
            StatusTone::Danger,
            "红色按钮会删除本机数据；备份和恢复只影响配置文件。",
        );
        ui.add_space(6.0);
        egui::Grid::new("maintenance_actions_grid")
            .num_columns(3)
            .striped(true)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(
                    RichText::new("配置")
                        .strong()
                        .size(SETTINGS_FONT_SETTING_TITLE)
                        .color(palette.text),
                );
                ui.label("备份或恢复 kaixin.ini。");
                ui.horizontal(|ui| {
                    if outline_button(ui, "备份").clicked() {
                        app.backup_config();
                    }
                    if outline_button(ui, "恢复").clicked() {
                        app.restore_config();
                    }
                });
                ui.end_row();

                ui.label(
                    RichText::new("用户词库")
                        .strong()
                        .size(SETTINGS_FONT_SETTING_TITLE)
                        .color(palette.text),
                );
                ui.label("清空本地学习词和上下文排序信号。");
                if danger_button(ui, "清空").clicked() {
                    app.clear_user_dict();
                }
                ui.end_row();

                ui.label(
                    RichText::new("日志")
                        .strong()
                        .size(SETTINGS_FONT_SETTING_TITLE)
                        .color(palette.text),
                );
                ui.label("清空 TSF 和运行时诊断日志。");
                if danger_button(ui, "清空").clicked() {
                    app.clear_tsf_log();
                }
                ui.end_row();
            });
    });

    ui.add_space(10.0);
    section_panel(ui, "敏感应用规则", |ui| {
        setting_toggle(
            ui,
            "全局隐私模式",
            "开启后强制 ASCII，不显示候选、不学习，并停止剪贴板捕获和历史读取。",
            &mut app.model.privacy_enabled,
        );
        ui.label("这些进程名单会以明文保存到 config.ini；需要隐藏使用痕迹时请留空。");
        privacy_process_list_row(
            ui,
            "永不学习",
            "匹配这些进程时，不写入用户词库、置顶、屏蔽和选择反馈。",
            &mut app.model.privacy_never_learn_processes,
        );
        privacy_process_list_row(
            ui,
            "永不候选",
            "匹配这些进程时，不查询或显示候选列表。",
            &mut app.model.privacy_never_candidate_processes,
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "通知", |ui| {
        setting_combo_row(
            ui,
            "通知显示",
            "状态切换和引擎提示是否弹出通知。",
            notification_label(&app.model.show_notifications).to_owned(),
            "notification_visibility",
            |ui| {
                selectable_string(ui, &mut app.model.show_notifications, "true", "开启");
                selectable_string(ui, &mut app.model.show_notifications, "false", "关闭");
                selectable_string(
                    ui,
                    &mut app.model.show_notifications,
                    "ime,full_shape,punct,fuzzy,double,app,engine",
                    "自定义种类",
                );
            },
        );
        setting_slider_usize(
            ui,
            "通知显示时长",
            "通知浮窗停留时间，单位毫秒。",
            &mut app.model.show_notifications_time,
            500..=5000,
        );
    });
}

pub(super) fn privacy_process_list_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut String,
) {
    let palette = fluent_palette(ui);
    ui.add_space(6.0);
    ui.label(
        RichText::new(title)
            .strong()
            .size(SETTINGS_FONT_SETTING_TITLE)
            .color(palette.text),
    );
    ui.label(RichText::new(description).small().color(palette.muted));
    ui.add(
        TextEdit::multiline(value)
            .desired_width(ui.available_width().max(320.0))
            .desired_rows(2)
            .hint_text("app.exe, password*.exe"),
    );
}

fn privacy_process_list_text(value: &str) -> String {
    let text = normalize_inline_list(value);
    if text.is_empty() {
        "未配置".to_string()
    } else {
        text
    }
}

pub(super) fn data_location_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    path: &Path,
    add_actions: impl FnOnce(&mut egui::Ui),
) {
    let palette = fluent_palette(ui);
    let row = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(0.0, 7.0))
        .show(ui, |ui| {
            let total_width = ui.available_width();
            let path_text = path.display().to_string();
            if total_width < SETTINGS_ROW_STACK_WIDTH {
                ui.vertical(|ui| {
                    ui.set_width(total_width);
                    ui.label(
                        RichText::new(title)
                            .strong()
                            .size(SETTINGS_FONT_SETTING_TITLE)
                            .color(palette.text),
                    );
                    ui.label(RichText::new(description).small().color(palette.muted));
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        let field_width =
                            (total_width - SETTINGS_BUTTON_WIDTH * 2.0 - 32.0).clamp(180.0, 420.0);
                        let mut readonly_path = path_text.clone();
                        ui.add_sized(
                            [field_width, 24.0],
                            TextEdit::singleline(&mut readonly_path)
                                .font(TextStyle::Monospace)
                                .interactive(false),
                        );
                        if outline_button(ui, "复制").clicked() {
                            ui.ctx().copy_text(path_text.clone());
                        }
                        add_actions(ui);
                    });
                });
            } else {
                let spacing = ui.spacing().item_spacing.x;
                let control_width = 560.0_f32.min((total_width * 0.54).max(390.0));
                let text_width = (total_width - control_width - spacing).max(180.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(50.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 50.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            ui.label(RichText::new(description).small().color(palette.muted));
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(control_width, 50.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(control_width);
                            ui.set_max_width(control_width);
                            let mut readonly_path = path_text;
                            let field_width = (control_width - SETTINGS_BUTTON_WIDTH * 2.0 - 24.0)
                                .clamp(180.0, 360.0);
                            ui.add_sized(
                                [field_width, 24.0],
                                TextEdit::singleline(&mut readonly_path)
                                    .font(TextStyle::Monospace)
                                    .interactive(false),
                            );
                            if outline_button(ui, "复制").clicked() {
                                ui.ctx().copy_text(readonly_path.clone());
                            }
                            add_actions(ui);
                        },
                    );
                });
            }
        });
    let y = row.response.rect.bottom();
    ui.painter().line_segment(
        [
            egui::pos2(row.response.rect.left(), y),
            egui::pos2(row.response.rect.right(), y),
        ],
        Stroke::new(1.0, palette.border_subtle),
    );
}

pub(super) fn folder_path_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut String,
    empty_hint: &str,
) {
    setting_row(ui, title, description, |ui| {
        let field_width = (ui.available_width() - SETTINGS_BUTTON_WIDTH - 16.0).clamp(180.0, 320.0);
        ui.add_sized(
            [field_width, 24.0],
            TextEdit::singleline(value).hint_text(empty_hint),
        );
        if outline_button(ui, "选择").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                *value = path.display().to_string();
            }
        }
    });
}

pub(super) fn executable_path_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut String,
    empty_hint: &str,
) {
    setting_row(ui, title, description, |ui| {
        let field_width = (ui.available_width() - 110.0).max(180.0);
        ui.add_sized(
            [field_width, 24.0],
            TextEdit::singleline(value).hint_text(empty_hint),
        );
        if outline_button(ui, "选择").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("WinTranslator", &["exe"])
                .pick_file()
            {
                *value = path.display().to_string();
            }
        }
        if !value.is_empty() && outline_button(ui, "清除").clicked() {
            value.clear();
        }
    });
}

pub(super) fn filename_pattern_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut String,
    empty_hint: &str,
) {
    setting_row(ui, title, description, |ui| {
        ui.add_sized(
            [260.0, 24.0],
            TextEdit::singleline(value).hint_text(empty_hint),
        );
    });
}

#[allow(dead_code)]
fn screenshot_dir_row(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    let row = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(0.0, 7.0))
        .show(ui, |ui| {
            let total_width = ui.available_width();
            if total_width < SETTINGS_ROW_STACK_WIDTH {
                ui.vertical(|ui| {
                    ui.set_width(total_width);
                    ui.label(
                        RichText::new("截图保存目录")
                            .strong()
                            .size(SETTINGS_FONT_SETTING_TITLE)
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(DEFAULT_SCREENSHOT_DIR_DESCRIPTION)
                            .small()
                            .color(palette.muted),
                    );
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        let field_width =
                            (total_width - SETTINGS_BUTTON_WIDTH - 20.0).clamp(180.0, 420.0);
                        ui.add_sized(
                            [field_width, 24.0],
                            TextEdit::singleline(&mut app.model.screenshot_save_dir)
                                .hint_text(DEFAULT_SCREENSHOT_DIR_HINT),
                        );
                        if outline_button(ui, "选择").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                app.model.screenshot_save_dir = path.display().to_string();
                            }
                        }
                    });
                });
            } else {
                let spacing = ui.spacing().item_spacing.x;
                let control_width = 430.0_f32.min((total_width * 0.44).max(320.0));
                let text_width = (total_width - control_width - spacing).max(180.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(50.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 50.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            ui.label(
                                RichText::new("截图保存目录")
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            ui.label(
                                RichText::new(DEFAULT_SCREENSHOT_DIR_DESCRIPTION)
                                    .small()
                                    .color(palette.muted),
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(control_width, 50.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(control_width);
                            ui.set_max_width(control_width);
                            let field_width =
                                (control_width - SETTINGS_BUTTON_WIDTH - 16.0).clamp(180.0, 320.0);
                            ui.add_sized(
                                [field_width, 24.0],
                                TextEdit::singleline(&mut app.model.screenshot_save_dir)
                                    .hint_text(DEFAULT_SCREENSHOT_DIR_HINT),
                            );
                            if outline_button(ui, "选择").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    app.model.screenshot_save_dir = path.display().to_string();
                                }
                            }
                        },
                    );
                });
            }
        });
    let y = row.response.rect.bottom();
    ui.painter().line_segment(
        [
            egui::pos2(row.response.rect.left(), y),
            egui::pos2(row.response.rect.right(), y),
        ],
        Stroke::new(1.0, palette.border_subtle),
    );
}

fn tsf_trace_log_path() -> PathBuf {
    runtime_log::log_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tsf.log")
}

pub(super) fn diagnostic_log_dir() -> PathBuf {
    runtime_log::log_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn compatibility_log_path() -> Option<PathBuf> {
    app_paths::local_data_dir().map(|dir| dir.join("compatibility.log"))
}

pub(crate) fn diagnostic_log_paths() -> Vec<PathBuf> {
    let mut paths = runtime_log::current_log_paths();
    if let Some(dir) = runtime_log::log_dir() {
        for name in runtime_log::known_log_files() {
            let path = dir.join(name);
            for index in 1..=runtime_log::LOG_ROTATE_KEEP {
                let rotated = runtime_log::rotated_log_path(&path, index);
                if !paths.iter().any(|existing| existing == &rotated) {
                    paths.push(rotated);
                }
            }
        }
    }
    if let Some(path) = compatibility_log_path() {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }
    paths
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UserDictEncryptionState {
    Encrypted,
    Plain,
    Missing,
}

fn user_dict_encryption_component_state(
    path: &Path,
) -> Result<UserDictEncryptionState, std::io::Error> {
    match pinyin_ime::user_dict_io::user_dict_file_is_encrypted(path) {
        Ok(true) => Ok(UserDictEncryptionState::Encrypted),
        Ok(false) => Ok(UserDictEncryptionState::Plain),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(UserDictEncryptionState::Missing)
        }
        Err(err) => Err(err),
    }
}

fn user_dict_encryption_status_text(path: &Path) -> String {
    let state = match user_dict_encryption_component_state(path) {
        Ok(state) => state,
        Err(err) => return format!("无法读取用户词库加密状态：{err}"),
    };
    let encryption_enabled = pinyin_ime::user_dict_io::user_dict_encryption_enabled();
    if !encryption_enabled {
        return if state == UserDictEncryptionState::Missing {
            "未启用：用户词库 SQLite 尚未生成。".to_string()
        } else {
            "未启用：用户词库 SQLite 以明文文件保存。".to_string()
        };
    }

    match state {
        UserDictEncryptionState::Encrypted => {
            "已加密：用户词库 SQLite 受 Windows DPAPI 保护。".to_string()
        }
        UserDictEncryptionState::Plain => {
            "需迁移：用户词库 SQLite 仍是明文或空文件；下次读取/写入会使用 Windows DPAPI。"
                .to_string()
        }
        UserDictEncryptionState::Missing => {
            "已启用：用户词库 SQLite 尚未生成，首次写入会使用 Windows DPAPI。".to_string()
        }
    }
}

pub(super) fn clipboard_storage_status_text(path: &Path) -> String {
    if !pinyin_ime::clipboard_store::clipboard_store_encryption_enabled() {
        return if path.is_file() {
            "本地 SQLite 数据库，当前平台未启用 DPAPI 加密。".to_string()
        } else {
            "本地 SQLite 数据库尚未生成；当前平台未启用 DPAPI 加密。".to_string()
        };
    }
    match pinyin_ime::clipboard_store::clipboard_store_file_is_encrypted(path) {
        Ok(true) => "已加密：本地 SQLite 数据库受 Windows DPAPI 保护。".to_string(),
        Ok(false) if path.is_file() => {
            "需迁移：剪贴板 SQLite 仍是明文或空文件；下次读取/写入会使用 Windows DPAPI。"
                .to_string()
        }
        Ok(false) => "已启用：剪贴板历史尚未生成，首次写入会使用 Windows DPAPI。".to_string(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            "已启用：剪贴板历史尚未生成，首次写入会使用 Windows DPAPI。".to_string()
        }
        Err(err) => format!("无法读取剪贴板加密状态：{err}"),
    }
}

pub(crate) fn privacy_statement_text(config_path: &Path, model: &SettingsModel) -> String {
    let data_dir = app_paths::local_data_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| ".".to_string());
    let user_dict_path = pinyin_ime::user_dict::default_user_dict_path();
    let clipboard_path = pinyin_ime::clipboard_store::store_path();
    let log_path = tsf_trace_log_path();
    let screenshot_dir = if model.screenshot_save_dir.trim().is_empty() {
        DEFAULT_SCREENSHOT_DIR_HINT.to_string()
    } else {
        model.screenshot_save_dir.trim().to_string()
    };
    let screenshot_copy_dir = if !model.screenshot_silent_copy_enabled {
        "未启用".to_string()
    } else if model.screenshot_silent_copy_dir.trim().is_empty() {
        "未设置（不会保存副本）".to_string()
    } else {
        model.screenshot_silent_copy_dir.trim().to_string()
    };
    let encryption_status = user_dict_encryption_status_text(&user_dict_path);
    let clipboard_storage_status = clipboard_storage_status_text(&clipboard_path);
    let never_learn_processes = privacy_process_list_text(&model.privacy_never_learn_processes);
    let never_clipboard_processes =
        privacy_process_list_text(&model.privacy_never_clipboard_processes);
    let never_candidate_processes =
        privacy_process_list_text(&model.privacy_never_candidate_processes);

    format!(
        "开心输入法隐私说明\n\
\n\
联网说明：本输入法不联网，不上传输入内容、候选、截图、剪贴板或用户词库。\n\
\n\
本地数据位置：\n\
- 数据目录：{data_dir}\n\
- 配置文件：{}\n\
- 用户词库：{}\n\
- 剪贴板历史：{}\n\
- TSF 诊断日志：{}\n\
- 截图保存目录：{screenshot_dir}\n\
- 截图副本目录：{screenshot_copy_dir}\n\
\n\
本地数据用途：\n\
- 用户词库用于学习上屏词、词频和上下文排序信号。\n\
- 剪贴板历史仅在后台保存剪贴板开启时写入本地 SQLite 数据库，Windows 上使用 DPAPI 保护。\n\
- TSF 日志默认只记录脱敏诊断和性能事件。\n\
- 截图文件只保存到用户选择的本地目录；开启静默副本时会额外复制到副本目录。\n\
\n\
用户词库加密状态：{encryption_status}\n\
剪贴板历史存储：{clipboard_storage_status}\n\
永不学习应用：{never_learn_processes}\n\
永不剪贴板应用：{never_clipboard_processes}\n\
永不候选应用：{never_candidate_processes}\n\
\n\
可清除数据：设置页可一键清空用户词库学习记录、剪贴板历史和 TSF 日志。\n",
        config_path.display(),
        user_dict_path.display(),
        clipboard_path.display(),
        log_path.display()
    )
}
