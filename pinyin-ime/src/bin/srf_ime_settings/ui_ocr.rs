use super::*;

pub(super) fn ocr_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    tool_page_intro(
        ui,
        "识",
        "本地文字识别",
        "截图、识别和结果处理都在本机完成。",
    );
    let mut open_ocr = false;
    let mut open_ocr_translate = false;
    let mut check_ocr = false;
    section_panel(ui, "本地 OCR", |ui| {
        setting_row(
            ui,
            "截图 OCR",
            "截取屏幕区域后由本地 RapidOCR 识别文字。",
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    open_ocr = true;
                }
                if outline_button(ui, "识别后翻译").clicked() {
                    open_ocr_translate = true;
                }
                if outline_button(ui, "检测").clicked() {
                    check_ocr = true;
                }
            },
        );
        if rapidocr_paths::rapidocr_root().is_none() {
            inline_notice(
                ui,
                StatusTone::Warning,
                "RapidOCR 环境未找到，OCR 暂不可用。点“检测”可查看缺失项。",
            );
        }
        setting_combo_row(
            ui,
            "OCR 完成后",
            "识别完成后的默认处理方式。",
            ocr_result_action_label(&app.model.ocr_result_action),
            "ocr_result_action",
            |ui| {
                selectable_string(ui, &mut app.model.ocr_result_action, "show", "仅显示结果");
                selectable_string(ui, &mut app.model.ocr_result_action, "copy", "自动复制");
                selectable_string(ui, &mut app.model.ocr_result_action, "paste", "自动粘贴");
            },
        );
        setting_toggle(
            ui,
            "OCR 模型常驻",
            "开启后复用模型以减少等待；关闭后释放内存。",
            &mut app.model.ocr_keep_alive,
        );
        setting_combo_row(
            ui,
            "OCR 速度与精度",
            "快速适合短文本；高精度使用更大的检测边长。",
            match app.model.ocr_profile.as_str() {
                "fast" => "快速",
                "accurate" => "高精度",
                _ => "均衡",
            },
            "ocr_profile",
            |ui| {
                selectable_string(
                    ui,
                    &mut app.model.ocr_profile,
                    "fast",
                    "快速（最长边 1280）",
                );
                selectable_string(
                    ui,
                    &mut app.model.ocr_profile,
                    "balanced",
                    "均衡（最长边 1920）",
                );
                selectable_string(
                    ui,
                    &mut app.model.ocr_profile,
                    "accurate",
                    "高精度（最长边 2560）",
                );
            },
        );
        setting_toggle(
            ui,
            "截图翻译后保留 OCR 窗口",
            "方便校对截图预览和识别文字。",
            &mut app.model.ocr_translate_keep_window,
        );
        setting_toggle(
            ui,
            "OCR 截图自动保存",
            "另存 OCR 的原始截图到本地目录。",
            &mut app.model.ocr_screenshot_auto_save,
        );
        folder_path_row(
            ui,
            "OCR 截图保存目录",
            "留空时使用“图片\\Kaixin OCR”。",
            &mut app.model.ocr_screenshot_save_dir,
            "默认：图片\\Kaixin OCR",
        );
        filename_pattern_row(
            ui,
            "OCR 截图命名规则",
            "支持 {date}、{time}、{datetime}、{seq}；不需要写扩展名。",
            &mut app.model.ocr_screenshot_name_pattern,
            "ocr-{datetime}",
        );
    });
    if open_ocr {
        app.open_ocr();
    }
    if open_ocr_translate {
        app.open_ocr_translate();
    }
    if check_ocr {
        app.check_ocr_language();
    }
}
