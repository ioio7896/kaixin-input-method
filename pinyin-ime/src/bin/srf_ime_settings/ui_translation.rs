use super::*;

pub(super) fn translation_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    tool_page_intro(
        ui,
        "译",
        "中英翻译",
        "统一管理翻译工具、路径和完成后的动作。",
    );
    let mut open_translate = false;
    let mut check_translate = false;
    section_panel(ui, "中英翻译", |ui| {
        setting_row(
            ui,
            "WinTranslator",
            "选中文本后发送到独立 WinTranslator，并自动开始翻译。",
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
                "未找到 WinTranslator。点“检测”可查看安装提示。",
            );
        }
        executable_path_row(
            ui,
            "WinTranslator 路径",
            "可留空自动检测；自定义安装目录时选择 WinTranslator.exe。",
            &mut app.model.wintranslator_path,
            "自动检测",
        );
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
