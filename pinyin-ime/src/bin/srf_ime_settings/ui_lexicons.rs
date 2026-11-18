//! Settings page controls.
use super::*;

pub(super) fn lexicon_learning_controls(ui: &mut egui::Ui, app: &mut SettingsApp) {
    setting_combo_row(
        ui,
        "学习灵敏度",
        "控制生词进入用户词库的速度。数字键或鼠标选词反馈更强；四字以上短语需重复使用，保守模式更少误学。",
        learning_sensitivity_label(&app.model.learning_sensitivity),
        "learning_sensitivity",
        |ui| {
            selectable_string(
                ui,
                &mut app.model.learning_sensitivity,
                schema_options::LEARNING_SENSITIVITY[0],
                "保守",
            );
            selectable_string(
                ui,
                &mut app.model.learning_sensitivity,
                schema_options::LEARNING_SENSITIVITY[1],
                "标准",
            );
            selectable_string(
                ui,
                &mut app.model.learning_sensitivity,
                schema_options::LEARNING_SENSITIVITY[2],
                "积极",
            );
        },
    );
    setting_combo_row(
        ui,
        "用户热词提前力度",
        "控制已经学过的单字、二字和三字 exact 用户词在候选栏里提前的强度。",
        user_hotword_boost_label(&app.model.user_hotword_boost),
        "user_hotword_boost",
        |ui| {
            selectable_string(
                ui,
                &mut app.model.user_hotword_boost,
                schema_options::USER_HOTWORD_BOOST[0],
                "保守",
            );
            selectable_string(
                ui,
                &mut app.model.user_hotword_boost,
                schema_options::USER_HOTWORD_BOOST[1],
                "标准",
            );
            selectable_string(
                ui,
                &mut app.model.user_hotword_boost,
                schema_options::USER_HOTWORD_BOOST[2],
                "强",
            );
            selectable_string(
                ui,
                &mut app.model.user_hotword_boost,
                schema_options::USER_HOTWORD_BOOST[3],
                "积极",
            );
        },
    );
}

pub(super) fn user_lexicon_controls(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.add_space(6.0);
    ui.label(
        RichText::new("自定义短语")
            .strong()
            .size(SETTINGS_FONT_SECTION_TITLE),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label("编码")
            .on_hover_text("输入自定义编码；输入该编码时可在候选栏选择对应短语。");
        ui.add(TextEdit::singleline(&mut app.user_phrase_key).desired_width(120.0))
            .on_hover_text("输入自定义编码；输入该编码时可在候选栏选择对应短语。");
        ui.label("短语")
            .on_hover_text("输入要加入用户词库的文字，点击“添加”保存。");
        ui.add(TextEdit::singleline(&mut app.user_phrase_text).desired_width(180.0))
            .on_hover_text("输入要加入用户词库的文字，点击“添加”保存。");
        if outline_button(ui, "添加").clicked() {
            app.add_user_phrase();
        }
    });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("永远不学")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        )
        .on_hover_text("加入后不再学习该短语；可从下方列表移出，恢复正常学习。");
        ui.add(TextEdit::singleline(&mut app.blocked_phrase_text).desired_width(180.0))
            .on_hover_text("输入不希望学习的短语，点击“加入”；点击“刷新”重新读取列表。");
        if outline_button(ui, "加入").clicked() {
            app.block_phrase_from_settings();
        }
        if outline_button(ui, "刷新").clicked() {
            app.load_blocked_phrases(true);
        }
    });
    if !app.blocked_phrases_loaded {
        app.load_blocked_phrases(false);
    }
    if app.blocked_phrases.is_empty() {
        ui.label(
            RichText::new("暂无永远不学项。")
                .small()
                .color(palette.muted),
        );
    } else {
        let rows: Vec<_> = app.blocked_phrases.iter().take(24).cloned().collect();
        let mut unblock_phrase: Option<String> = None;
        egui::Grid::new("blocked_phrases_grid")
            .num_columns(2)
            .striped(true)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label(RichText::new("短语").strong().color(palette.text));
                ui.label(RichText::new("操作").strong().color(palette.text));
                ui.end_row();

                for item in rows {
                    ui.label(&item.phrase);
                    if outline_button(ui, "移出").clicked() {
                        unblock_phrase = Some(item.phrase.clone());
                    }
                    ui.end_row();
                }
            });
        if app.blocked_phrases.len() > 24 {
            ui.label(
                RichText::new("仅显示最近 24 条。")
                    .small()
                    .color(palette.muted),
            );
        }
        if let Some(phrase) = unblock_phrase {
            app.unblock_phrase_from_settings(&phrase);
        }
    }
}

pub(super) fn extension_lexicon_controls(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    if !app.model.lexicon_tags.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new("扩展词库")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        )
        .on_hover_text("基础词库与读音修正始终启用；扩展词库按需选择，保存设置后生效。");
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.lexicon_filter)
                    .hint_text("搜索词库名称、用途或文件标识")
                    .desired_width(280.0),
            );
            if outline_button(ui, "恢复词库默认").clicked() {
                for (tag, enabled) in &mut app.model.lexicon_tags {
                    *enabled = pinyin_ime::lexicon_prefs::default_optional_lexicon_tag_enabled(tag);
                }
            }
            if outline_button(ui, "刷新列表与统计").clicked() {
                merge_discovered_lexicon_tags_compat(&mut app.model);
                app.lexicon_catalog = None;
                app.lexicon_catalog_rx = None;
            }
        });
        if app.lexicon_catalog.is_none() && app.lexicon_catalog_rx.is_none() {
            if let Some(root) = pinyin_ime::core::default_phrase_lexicon_dir() {
                let (tx, rx) = std::sync::mpsc::channel();
                app.lexicon_catalog_rx = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send(pinyin_ime::lexicon_prefs::discover_optional_lexicon_info(
                        &root,
                    ));
                });
            }
        }
        if let Some(rx) = &app.lexicon_catalog_rx {
            match rx.try_recv() {
                Ok(catalog) => {
                    app.lexicon_catalog = Some(catalog);
                    app.lexicon_catalog_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.lexicon_catalog = Some(Vec::new());
                    app.lexicon_catalog_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        let filter = app.lexicon_filter.trim().to_lowercase();
        let mut visible = 0;
        for group in [
            "日常生活",
            "办公沟通",
            "科技开发",
            "专业领域",
            "名称与地区",
            "通用补充",
            "其他扩展",
        ] {
            let mut all_tags: Vec<_> = app
                .model
                .lexicon_tags
                .keys()
                .filter(|tag| lexicon_display_group(tag) == group)
                .cloned()
                .collect();
            all_tags.sort_by(|a, b| lexicon_sort_key(a).cmp(&lexicon_sort_key(b)));
            let tags: Vec<_> = all_tags
                .iter()
                .filter(|tag| {
                    format!(
                        "{} {} {} {}",
                        lexicon_tag_label(tag),
                        tag,
                        group,
                        lexicon_coverage(tag)
                    )
                    .to_lowercase()
                    .contains(&filter)
                })
                .cloned()
                .collect();
            if tags.is_empty() {
                continue;
            }
            visible += tags.len();
            let enabled_count = all_tags
                .iter()
                .filter(|tag| app.model.lexicon_tags[*tag])
                .count();
            // The identity excludes live counts so changing a switch preserves expansion.
            egui::CollapsingHeader::new(format!(
                "{group} · 已开启 {enabled_count} / {}",
                all_tags.len()
            ))
            .id_salt(("lexicon_group", group))
            .default_open(false)
            .open(if filter.is_empty() { None } else { Some(true) })
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if outline_button(ui, "全选本组")
                        .on_hover_text("包含被搜索条件隐藏的本组词库；保存设置后生效。")
                        .clicked()
                    {
                        set_lexicon_group_enabled(&mut app.model.lexicon_tags, &all_tags, true);
                    }
                    if outline_button(ui, "取消本组")
                        .on_hover_text("包含被搜索条件隐藏的本组词库；保存设置后生效。")
                        .clicked()
                    {
                        set_lexicon_group_enabled(&mut app.model.lexicon_tags, &all_tags, false);
                    }
                    if tags.len() != all_tags.len() {
                        ui.label(
                            RichText::new(format!(
                                "筛选显示 {} 项；操作作用于本组全部",
                                tags.len()
                            ))
                            .small()
                            .color(palette.muted),
                        );
                    }
                });
                ui.add_space(4.0);
                for tag in tags {
                    let info = app
                        .lexicon_catalog
                        .as_ref()
                        .and_then(|items| items.iter().find(|item| item.tag == tag));
                    let count = match info {
                        Some(info) if !info.incomplete => format!("{} 条", info.entries),
                        Some(_) => "统计不完整".to_string(),
                        None if app.lexicon_catalog_rx.is_some() => "统计中…".to_string(),
                        None => "统计不可用".to_string(),
                    };
                    let source = info
                        .map(|info| info.files.join("\n"))
                        .unwrap_or_else(|| "正在读取词库来源。".to_string());
                    if let Some(enabled) = app.model.lexicon_tags.get_mut(&tag) {
                        lexicon_list_row(
                            ui,
                            lexicon_tag_label(&tag),
                            &count,
                            enabled,
                            lexicon_coverage(&tag),
                            &source,
                        );
                    }
                }
                ui.add_space(12.0);
            })
            .header_response
            .on_hover_text(lexicon_group_hint(group));
        }
        if visible == 0 {
            ui.label("没有匹配的扩展词库。");
        }
    } else {
        ui.label(
            RichText::new("暂无扩展词库；可以导入词库文件，或重新加载列表。").color(palette.muted),
        );
    }
    ui.horizontal_wrapped(|ui| {
        if outline_button(ui, "导入词库").clicked() {
            app.import_lexicon_text();
        }
        if outline_button(ui, "重新加载").clicked() {
            app.reload_lexicon_now();
        }
        ui.label(
            RichText::new("导入或替换词库文件后，可在这里立即重新加载。")
                .small()
                .color(palette.muted),
        );
    });
}
