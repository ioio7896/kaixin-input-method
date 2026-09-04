use super::*;

pub(super) fn screenshot_settings_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    tool_page_intro(
        ui,
        "截",
        "截图工具",
        "用一致的快捷操作完成截图、保存和 OCR 衔接。",
    );
    section_panel(ui, "截图设置", |ui| {
        let palette = fluent_palette(ui);
        setting_combo_row(
            ui,
            "截图方案",
            "使用内置原生截图，可智能吸附窗口和控件，也可自由拖动框选。",
            screenshot_capture_scheme_label(&app.model.screenshot_mode).to_owned(),
            "screenshot_scheme",
            |ui| {
                if ui
                    .selectable_label(app.model.screenshot_mode == "manual_region", "智能框选")
                    .on_hover_text("直接使用系统原生 GPU 捕获路径。")
                    .clicked()
                {
                    app.model.screenshot_mode = "manual_region".to_string();
                }
                if ui
                    .selectable_label(app.model.screenshot_mode == "current_window", "当前窗口")
                    .on_hover_text("直接捕获当前窗口，适合只想快速保存或复制的场景。")
                    .clicked()
                {
                    app.model.screenshot_mode = "current_window".to_string();
                }
            },
        );
        ui.label(
            RichText::new(
                "受保护视频、DRM 和安全桌面无法截取；WGC 不可用时自动回退 DXGI 或系统截图。",
            )
            .small()
            .color(palette.muted),
        );
        if app.model.screenshot_mode == "manual_region" {
            setting_toggle(
                ui,
                "松开鼠标立即完成",
                "开启后，自由拖动框选并松开鼠标时立即提交截图；关闭后可继续移动或缩放选区，再点击“完成”、双击选区或按 Enter。",
                &mut app.model.screenshot_confirm_on_release,
            );
            setting_toggle(
                ui,
                "显示框选操作提示",
                "在框选界面顶部显示吸附、快捷键和确认方式；选区旁的“完成/取消”按钮始终显示。",
                &mut app.model.screenshot_show_instructions,
            );
            ui.label(
                RichText::new(if app.model.screenshot_confirm_on_release {
                    "当前确认方式：拖动松手后立即截图；单击智能吸附区域同样会立即完成。"
                } else {
                    "当前确认方式：拖动松手后保留选区，可调整范围，再点击“完成”、双击或按 Enter。"
                })
                .small()
                .color(palette.muted),
            );
        }
        ui.add_space(6.0);
        ui.label(RichText::new("截图完成后").strong());
        setting_toggle(
            ui,
            "保存图片",
            "把最终截图写入本地截图库；OCR 和翻译仍会接收明确的图片路径。",
            &mut app.model.screenshot_auto_save,
        );
        setting_toggle(
            ui,
            "复制图片",
            "把最终截图复制到系统剪贴板，方便直接粘贴到聊天或文档。",
            &mut app.model.screenshot_copy_after_capture,
        );
        setting_toggle(
            ui,
            "自动 OCR",
            "截图完成后把实际图片文件直接交给 OCR，并立即显示预览和识别进度；不依赖剪贴板。",
            &mut app.model.screenshot_ocr_after_capture,
        );
        setting_toggle(
            ui,
            "OCR 后翻译",
            "识别完成后自动打开本地中英翻译；开启时会同时启用自动 OCR。",
            &mut app.model.screenshot_translate_after_capture,
        );
        if app.model.screenshot_translate_after_capture {
            app.model.screenshot_ocr_after_capture = true;
        }
        if !app.model.screenshot_auto_save
            && !app.model.screenshot_copy_after_capture
            && !app.model.screenshot_ocr_after_capture
            && !app.model.screenshot_translate_after_capture
        {
            ui.label(
                RichText::new("请至少开启保存、复制、OCR 或翻译中的一项，否则截图结果不会被保留。")
                    .small()
                    .color(palette.danger),
            );
        }
        folder_path_row(
            ui,
            "截图保存目录",
            DEFAULT_SCREENSHOT_DIR_DESCRIPTION,
            &mut app.model.screenshot_save_dir,
            DEFAULT_SCREENSHOT_DIR_HINT,
        );
        setting_toggle(
            ui,
            "静默保存截图副本",
            "开启后，截图自动保存成功时会额外复制一份到副本目录；复制失败只写日志。",
            &mut app.model.screenshot_silent_copy_enabled,
        );
        folder_path_row(
            ui,
            "截图副本目录",
            "仅在静默保存副本开启且目录不为空时使用。",
            &mut app.model.screenshot_silent_copy_dir,
            "选择副本目录",
        );
        filename_pattern_row(
            ui,
            "截图命名规则",
            "支持 {timestamp}（含毫秒）、{date}、{time}、{datetime}、{seq}、{app}、{window}、{width}、{height}；不需要写扩展名。",
            &mut app.model.screenshot_name_pattern,
            "{timestamp}",
        );
        setting_toggle(
            ui,
            "按日期自动分目录",
            "在保存目录下按 YYYY/MM/DD 建立子目录，便于长期整理截图。",
            &mut app.model.screenshot_date_subdirs,
        );
        setting_combo_row(
            ui,
            "文件冲突策略",
            "同名截图已存在时选择覆盖，或自动递增为 _002、_003。",
            if app.model.screenshot_conflict_strategy == "overwrite" {
                "覆盖"
            } else {
                "自动递增"
            }
            .to_owned(),
            "screenshot_conflict_strategy",
            |ui| {
                selectable_string(
                    ui,
                    &mut app.model.screenshot_conflict_strategy,
                    "increment",
                    "自动递增",
                );
                selectable_string(
                    ui,
                    &mut app.model.screenshot_conflict_strategy,
                    "overwrite",
                    "覆盖",
                );
            },
        );
        setting_combo_row(
            ui,
            "截图格式",
            "托盘截图自动保存的图片格式。",
            screenshot_format_label(&app.model.screenshot_format).to_owned(),
            "screenshot_format",
            |ui| {
                selectable_string(ui, &mut app.model.screenshot_format, "png", "PNG");
                selectable_string(ui, &mut app.model.screenshot_format, "jpg", "JPG");
            },
        );
    });
}
