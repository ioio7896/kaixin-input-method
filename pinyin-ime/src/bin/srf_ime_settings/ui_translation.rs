use super::*;

pub(super) fn translation_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let mut open_translate = false;
    let mut check_translate = false;
    section_panel(ui, "本地翻译（HY-MT2 联动）", |ui| {
        setting_row(
            ui,
            "HY-MT2 / WinTranslator",
            "候选（Ctrl+Shift+Enter / vv tr）、选区和 OCR 共用常驻流式翻译预览；支持双工实时生成、术语表直加与无痕回填。",
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    open_translate = true;
                }
                if outline_button(ui, "检测").clicked() {
                    check_translate = true;
                }
                if outline_button(ui, "预加载引擎").clicked() {
                    external_translation::preload_translator_async();
                    app.status = "已向 HY-MT2 发送本地模型预加载请求。".into();
                }
            },
        );
        if !translation_available() {
            inline_notice(
                ui,
                StatusTone::Warning,
                "未找到翻译服务。请安装 HY-MT2 翻译机或选择程序路径。",
            );
        }
        executable_path_row(
            ui,
            "翻译程序路径",
            "可留空自动检测 HY-MT2（支持 HKCU/HKLM 注册表与默认安装目录）；仍兼容 WinTranslator.exe。",
            &mut app.model.wintranslator_path,
            "自动检测",
        );
        let languages = external_translation::supported_languages();
        setting_combo_row(
            ui,
            "目标语言",
            "已同步 HY-MT2 全量 38 种语言；自动模式支持中英互译与代码/标识符过滤。",
            external_translation::language_label(&app.model.translate_target_language),
            "translate_target_language",
            |ui| {
                for (code, label) in &languages {
                    selectable_string(
                        ui,
                        &mut app.model.translate_target_language,
                        code.as_str(),
                        label.as_str(),
                    );
                }
            },
        );
        setting_combo_row(
            ui,
            "翻译完成后",
            "候选、选区和 OCR 的默认操作；短单行优先直输不改动剪贴板，长文粘贴后自动恢复原剪贴板。",
            translate_result_action_label(&app.model.translate_result_action),
            "translate_result_action",
            |ui| {
                selectable_string(
                    ui,
                    &mut app.model.translate_result_action,
                    "show",
                    "预览后确认输入",
                );
                selectable_string(
                    ui,
                    &mut app.model.translate_result_action,
                    "copy",
                    "自动复制",
                );
                selectable_string(
                    ui,
                    &mut app.model.translate_result_action,
                    "paste",
                    "自动粘贴",
                );
            },
        );
        let mut continuous = external_translation::continuous_translate_enabled();
        setting_row(
            ui,
            "连续翻译模式（中打英出）",
            "开启后，输入中文词组/整句并提交时将自动弹出翻译预览（也可在预览窗右上角随时开关）。",
            |ui| {
                if ui.checkbox(&mut continuous, "启用连续翻译").changed() {
                    match external_translation::set_continuous_translate_enabled(continuous) {
                        Ok(()) => {
                            app.status = if continuous {
                                "已启用连续翻译模式（中打英出）。".into()
                            } else {
                                "已关闭连续翻译模式。".into()
                            };
                        }
                        Err(err) => app.status = format!("切换连续翻译模式失败：{err}"),
                    }
                }
                if outline_button(ui, "取消后台翻译").clicked() {
                    app.status = format!(
                        "已请求取消 {} 个后台会话。",
                        external_translation::cancel_pending_translations()
                    );
                }
            },
        );
        inline_notice(
            ui,
            StatusTone::Info,
            "建议分工：候选/选区/截图 OCR 翻译走开心输入法（内置 RapidOCR），网页整页与长文编辑走 HY-MT2。请注意不要与 HY-MT2 默认快捷键 Ctrl+Alt+T 冲突。",
        );
    });

    section_panel(ui, "按应用保存的翻译偏好", |ui| {
        let app_prefs = external_translation::load_app_preferences();
        if app_prefs.is_empty() {
            ui.label(
                "尚未保存特定应用的翻译偏好。在翻译预览中勾选「记住应用」并保存后会显示在此处。",
            );
        } else {
            let mut remove_key: Option<String> = None;
            let mut clear_all = false;
            for (exe, pref) in &app_prefs {
                let name = std::path::Path::new(exe)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(exe.as_str());
                let lang = external_translation::language_label(&pref.language);
                let act = translate_result_action_label(&pref.action);
                setting_row(
                    ui,
                    name,
                    &format!("目标语言：{lang} · 完成后：{act}（{exe}）"),
                    |ui| {
                        if outline_button(ui, "删除").clicked() {
                            remove_key = Some(exe.clone());
                        }
                    },
                );
            }
            if outline_button(ui, "清空全部应用偏好").clicked() {
                clear_all = true;
            }
            if let Some(key) = remove_key {
                app.status = match external_translation::save_app_preference(&key, None) {
                    Ok(()) => "已删除该应用的翻译偏好。".into(),
                    Err(err) => format!("删除应用偏好失败：{err}"),
                };
            } else if clear_all {
                app.status = match external_translation::clear_app_preferences() {
                    Ok(()) => "已清空全部应用翻译偏好。".into(),
                    Err(err) => format!("清空应用偏好失败：{err}"),
                };
            }
        }
    });

    if open_translate {
        app.open_translate();
    }
    if check_translate {
        app.check_translation_environment();
    }
}
