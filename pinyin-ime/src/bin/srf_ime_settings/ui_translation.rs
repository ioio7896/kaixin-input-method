use super::*;

pub(super) fn translation_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    tool_page_intro(
        ui,
        "译",
        "本地翻译",
        "统一管理翻译工具、路径和完成后的动作。",
    );
    let mut open_translate = false;
    let mut check_translate = false;
    section_panel(ui, "本地翻译", |ui| {
        setting_row(
            ui,
            "HY-MT2 / WinTranslator",
            "候选、选区和 OCR 文本交给独立 HY-MT2 翻译机或 WinTranslator，在本机翻译。",
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    open_translate = true;
                }
                if outline_button(ui, "检测").clicked() {
                    check_translate = true;
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
            "可留空自动检测 HY-MT2；自定义目录选择 hy-mt2-desktop.exe，仍兼容 WinTranslator.exe。",
            &mut app.model.wintranslator_path,
            "自动检测",
        );
        setting_combo_row(
            ui,
            "目标语言",
            "自动模式以中英互译为主；日文和混合文本可指定目标语言。",
            &app.model.translate_target_language.clone(),
            "translate_target_language",
            |ui| {
                for (code, label) in [
                    ("auto-opposite", "自动中英互译"),
                    ("zh", "简体中文"),
                    ("zh-Hant", "繁体中文"),
                    ("en", "英语"),
                    ("ja", "日语"),
                    ("ko", "韩语"),
                    ("fr", "法语"),
                    ("de", "德语"),
                    ("es", "西班牙语"),
                    ("ru", "俄语"),
                ] {
                    selectable_string(ui, &mut app.model.translate_target_language, code, label);
                }
            },
        );
        if outline_button(ui, "取消后台翻译").clicked() {
            app.status = format!(
                "已请求取消 {} 个后台会话。",
                external_translation::cancel_pending_translations()
            );
        }
        setting_combo_row(
            ui,
            "翻译完成后",
            "用翻译热键或剪贴板启动后，译文的默认处理方式。",
            translate_result_action_label(&app.model.translate_result_action),
            "translate_result_action",
            |ui| {
                selectable_string(
                    ui,
                    &mut app.model.translate_result_action,
                    "show",
                    "仅显示结果",
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
    });
    if open_translate {
        app.open_translate();
    }
    if check_translate {
        app.check_translation_environment();
    }
}
