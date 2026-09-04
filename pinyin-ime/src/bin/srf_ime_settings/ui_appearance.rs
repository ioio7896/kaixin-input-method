use super::*;

fn candidate_preview_colors(
    ui: &egui::Ui,
    model: &SettingsModel,
    skins: &[SkinPreview],
) -> CandidatePreviewColors {
    let palette = fluent_palette(ui);
    let selected = selected_skin(model, skins);
    if selected.is_some() {
        return preview_colors_for_skin(palette, selected);
    }
    if model.theme == "high_contrast" {
        return CandidatePreviewColors {
            window: Color32::BLACK,
            header: Color32::BLACK,
            border: Color32::WHITE,
            divider: Color32::WHITE,
            item: Color32::BLACK,
            item_border: Color32::WHITE,
            selected: Color32::from_rgb(255, 242, 0),
            selected_border: Color32::WHITE,
            text: Color32::WHITE,
            muted: Color32::from_gray(210),
            selected_text: Color32::BLACK,
            selected_muted: Color32::BLACK,
            chip: Color32::BLACK,
            chip_border: Color32::WHITE,
            chip_text: Color32::WHITE,
        };
    }
    if model.theme == "dark" {
        return CandidatePreviewColors {
            window: Color32::from_rgb(31, 35, 42),
            header: Color32::from_rgb(48, 55, 66),
            border: Color32::from_rgb(72, 78, 90),
            divider: Color32::from_rgb(72, 78, 90),
            item: Color32::from_rgb(38, 43, 52),
            item_border: Color32::from_rgb(72, 78, 90),
            selected: Color32::from_rgb(39, 73, 68),
            selected_border: Color32::from_rgb(76, 209, 151),
            text: Color32::from_rgb(244, 247, 250),
            muted: Color32::from_rgb(184, 193, 204),
            selected_text: Color32::WHITE,
            selected_muted: Color32::from_rgb(230, 235, 241),
            chip: Color32::from_rgb(48, 55, 66),
            chip_border: Color32::from_rgb(72, 78, 90),
            chip_text: Color32::from_rgb(230, 235, 241),
        };
    }
    if model.theme == "light" {
        return CandidatePreviewColors {
            window: Color32::from_rgb(248, 250, 252),
            header: Color32::from_rgb(241, 245, 249),
            border: Color32::from_rgb(203, 213, 225),
            divider: Color32::from_rgb(226, 232, 240),
            item: Color32::WHITE,
            item_border: Color32::from_rgb(226, 232, 240),
            selected: Color32::from_rgb(226, 246, 240),
            selected_border: Color32::from_rgb(0, 137, 110),
            text: Color32::from_rgb(15, 23, 42),
            muted: Color32::from_rgb(100, 116, 139),
            selected_text: Color32::from_rgb(15, 23, 42),
            selected_muted: Color32::from_rgb(71, 85, 105),
            chip: Color32::from_rgb(241, 245, 249),
            chip_border: Color32::from_rgb(226, 232, 240),
            chip_text: Color32::from_rgb(51, 65, 85),
        };
    }
    preview_colors_for_skin(palette, None)
}

fn with_preview_opacity(color: Color32, opacity: usize) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        ((opacity.clamp(10, 100) as f32 / 100.0) * 255.0).round() as u8,
    )
}

#[derive(Clone, Copy, Debug)]
struct CandidatePreviewLayoutSpec {
    outer_pad_y: f32,
    header_pad_y: f32,
    header_gap: f32,
    item_gap: f32,
    item_pad_y: f32,
    label_width: f32,
    comment_gap: f32,
}

#[derive(Clone, Copy, Debug)]
struct CandidatePreviewMetrics {
    height: f32,
    outer_pad_y: f32,
    header_height: f32,
    header_gap: f32,
    item_gap: f32,
    item_height: f32,
    count: usize,
    has_comment: bool,
}

fn candidate_preview_layout_spec(
    model: &SettingsModel,
    skins: &[SkinPreview],
) -> CandidatePreviewLayoutSpec {
    let layout = if model.candidate_horizontal {
        model.candidate_horizontal_layout_variant.as_str()
    } else {
        model.candidate_vertical_layout_variant.as_str()
    };
    let mut spec = match (layout, model.candidate_horizontal) {
        ("compact", true) => CandidatePreviewLayoutSpec {
            outer_pad_y: 5.0,
            header_pad_y: 5.0,
            header_gap: 3.0,
            item_gap: 1.0,
            item_pad_y: 4.0,
            label_width: 22.0,
            comment_gap: 2.0,
        },
        ("compact", false) => CandidatePreviewLayoutSpec {
            outer_pad_y: 7.0,
            header_pad_y: 6.0,
            header_gap: 5.0,
            item_gap: 3.0,
            item_pad_y: 6.0,
            label_width: 28.0,
            comment_gap: 4.0,
        },
        ("card", _) => CandidatePreviewLayoutSpec {
            outer_pad_y: 8.0,
            header_pad_y: 8.0,
            header_gap: 6.0,
            item_gap: 3.0,
            item_pad_y: 6.0,
            label_width: 28.0,
            comment_gap: 3.0,
        },
        _ => CandidatePreviewLayoutSpec {
            outer_pad_y: 6.0,
            header_pad_y: 5.0,
            header_gap: 4.0,
            item_gap: 2.0,
            item_pad_y: 5.0,
            label_width: 26.0,
            comment_gap: 3.0,
        },
    };

    if let Some(skin) = selected_skin(model, skins) {
        if let Some(value) = skin.outer_pad_y {
            spec.outer_pad_y = value;
        }
        if let Some(value) = skin.header_pad_y {
            spec.header_pad_y = value;
        }
        if let Some(value) = skin.header_gap {
            spec.header_gap = value;
        }
        if let Some(value) = skin.item_gap {
            spec.item_gap = value;
        }
        if let Some(value) = skin.item_pad_y {
            spec.item_pad_y = value;
        }
        if let Some(value) = skin.label_width {
            spec.label_width = value;
        }
        if let Some(value) = skin.comment_gap {
            spec.comment_gap = value;
        }
    }

    match model.candidate_density.as_str() {
        "compact" => {
            spec.outer_pad_y = (spec.outer_pad_y - 2.0).max(4.0);
            spec.header_pad_y = (spec.header_pad_y - 1.0).max(4.0);
            spec.header_gap = (spec.header_gap - 1.0).max(2.0);
            spec.item_gap = (spec.item_gap - 1.0).max(1.0);
            spec.item_pad_y = (spec.item_pad_y - 1.0).max(3.0);
            spec.label_width = (spec.label_width - 2.0).max(20.0);
            spec.comment_gap = (spec.comment_gap - 1.0).max(1.0);
        }
        "comfortable" => {
            spec.outer_pad_y += 1.0;
            spec.header_pad_y += 1.0;
            spec.header_gap += 1.0;
            spec.item_gap += 1.0;
            spec.item_pad_y += 1.0;
            spec.label_width += 2.0;
            spec.comment_gap += 1.0;
        }
        _ => {}
    }
    spec
}

fn candidate_preview_metrics(
    model: &SettingsModel,
    skins: &[SkinPreview],
) -> CandidatePreviewMetrics {
    let spec = candidate_preview_layout_spec(model, skins);
    let horizontal = model.candidate_horizontal;
    let skin_loaded = selected_skin(model, skins).is_some();
    let horizontal_compact_delta = if horizontal {
        (if model.candidate_horizontal_compact {
            2.0
        } else {
            0.0
        }) + if skin_loaded { 1.0 } else { 0.0 }
    } else {
        0.0
    };
    let outer_pad_y = if horizontal {
        (spec.outer_pad_y - horizontal_compact_delta).max(0.0)
    } else {
        spec.outer_pad_y
    };
    let item_pad_y = if horizontal {
        (spec.item_pad_y - horizontal_compact_delta).max(0.0)
    } else {
        spec.item_pad_y
    };
    let has_comment =
        model.show_candidate_source || model.show_candidate_reading || model.show_candidate_score;
    let body_line_height = (model.candidate_font_size as f32 * 1.25).max(18.0);
    let meta_font_size = model.candidate_font_size.saturating_sub(2).max(9) as f32;
    let meta_line_height = (meta_font_size * 1.25).max(12.0);
    let content_height = item_pad_y * 2.0
        + body_line_height
        + if has_comment {
            spec.comment_gap + meta_line_height
        } else {
            0.0
        };
    let item_height = if horizontal {
        content_height.max(if has_comment { 42.0 } else { 30.0 })
    } else {
        content_height
            .max(item_pad_y * 2.0 + 22.0_f32.max(spec.label_width))
            .max(30.0)
    };
    let count = if horizontal {
        model.candidate_horizontal_count.clamp(3, 9)
    } else {
        model.candidate_page_size.clamp(3, 10)
    };
    let header_height = if horizontal {
        0.0
    } else {
        ((model.candidate_font_size + 1) as f32 * 1.25 + spec.header_pad_y * 2.0).max(32.0)
    };
    let header_gap = if horizontal { 0.0 } else { spec.header_gap };
    let rows_height = if horizontal {
        item_height
    } else {
        item_height * count as f32 + spec.item_gap * count.saturating_sub(1) as f32
    };
    CandidatePreviewMetrics {
        height: outer_pad_y * 2.0 + header_height + header_gap + rows_height,
        outer_pad_y,
        header_height,
        header_gap,
        item_gap: spec.item_gap,
        item_height,
        count,
        has_comment,
    }
}

fn candidate_live_preview(ui: &mut egui::Ui, model: &SettingsModel, skins: &[SkinPreview]) {
    let colors = candidate_preview_colors(ui, model, skins);
    let horizontal = model.candidate_horizontal;
    let width = ui.available_width().min(720.0);
    let metrics = candidate_preview_metrics(model, skins);
    let height = metrics.height;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    let window = with_preview_opacity(colors.window, model.candidate_opacity);
    painter.rect(rect, 9.0, window, Stroke::new(1.0, colors.border));

    let content = rect.shrink2(egui::vec2(9.0, metrics.outer_pad_y));
    let body_top = if horizontal {
        content.top()
    } else {
        let header = egui::Rect::from_min_size(
            content.min,
            egui::vec2(content.width(), metrics.header_height),
        );
        painter.rect(
            header,
            6.0,
            with_preview_opacity(colors.header, model.candidate_opacity),
            Stroke::new(1.0, colors.divider),
        );
        painter.text(
            egui::pos2(header.left() + 4.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            "shurufa",
            FontId::proportional(SETTINGS_FONT_SMALL.max(SETTINGS_MIN_HINT_FONT)),
            colors.muted,
        );
        if model.show_mode_in_candidate_header {
            let chip = egui::Rect::from_min_size(
                egui::pos2(header.right() - 40.0, header.center().y - 11.0),
                egui::vec2(36.0, 22.0),
            );
            painter.rect(chip, 6.0, colors.chip, Stroke::new(1.0, colors.chip_border));
            painter.text(
                chip.center(),
                egui::Align2::CENTER_CENTER,
                "中文",
                FontId::proportional(SETTINGS_MIN_HINT_FONT.max(11.0)),
                colors.chip_text,
            );
        }
        header.bottom() + metrics.header_gap
    };

    let candidates = [
        ("输入法", "常用"),
        ("输入", "全拼"),
        ("输入方式", "词库"),
        ("输入符号", "联想"),
        ("输入方案", "用户词"),
        ("输入体验", "常用"),
        ("输入习惯", "全拼"),
        ("输入工具", "词库"),
        ("输入状态", "联想"),
        ("输入设置", "用户词"),
    ];
    let body =
        egui::Rect::from_min_max(egui::pos2(content.left(), body_top), content.right_bottom());
    let font_size = (model.candidate_font_size as f32).clamp(12.0, 28.0);
    let meta_font_size = (font_size * 0.75).clamp(9.0, 13.0);
    let count = metrics.count;

    let paint_row = |painter: &egui::Painter,
                     idx: usize,
                     candidate: &str,
                     meta: &str,
                     row: egui::Rect,
                     selected: bool| {
        let fill = if selected {
            colors.selected
        } else {
            colors.item
        };
        let stroke = if selected {
            Stroke::new(1.0, colors.selected_border)
        } else {
            Stroke::new(0.0, Color32::TRANSPARENT)
        };
        painter.rect(row, 6.0, fill, stroke);
        if selected {
            painter.rect(
                egui::Rect::from_min_max(
                    row.left_top(),
                    egui::pos2(row.left() + 4.0, row.bottom()),
                ),
                2.0,
                colors.selected_border,
                Stroke::new(0.0, Color32::TRANSPARENT),
            );
            painter.circle_filled(
                egui::pos2(row.left() + 12.0, row.center().y),
                9.0,
                colors.selected_border,
            );
            painter.text(
                egui::pos2(row.left() + 12.0, row.center().y),
                egui::Align2::CENTER_CENTER,
                format!("{}", idx + 1),
                FontId::proportional(11.0),
                colors.selected_text,
            );
        } else {
            painter.text(
                egui::pos2(row.left() + 10.0, row.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{}", idx + 1),
                FontId::proportional(11.0),
                colors.muted,
            );
        }
        let text_left = row.left() + 28.0;
        painter.text(
            egui::pos2(text_left, row.top() + 7.0),
            egui::Align2::LEFT_TOP,
            candidate,
            FontId::proportional(font_size),
            if selected {
                colors.selected_text
            } else {
                colors.text
            },
        );
        if metrics.has_comment {
            painter.text(
                egui::pos2(row.left() + 28.0, row.bottom() - 6.0),
                egui::Align2::LEFT_BOTTOM,
                meta,
                FontId::proportional(meta_font_size),
                if selected {
                    colors.selected_muted
                } else {
                    colors.muted
                },
            );
        }
    };

    if horizontal {
        let gap = if model.candidate_horizontal_compact {
            3.0
        } else {
            6.0
        };
        let card_width = (body.width() - gap * (count.saturating_sub(1)) as f32) / count as f32;
        for (idx, (candidate, meta)) in candidates.iter().take(count).enumerate() {
            let card = egui::Rect::from_min_size(
                egui::pos2(body.left() + idx as f32 * (card_width + gap), body.top()),
                egui::vec2(card_width, body.height()),
            );
            let selected = idx == 0;
            paint_row(&painter, idx, candidate, meta, card, selected);
        }
    } else {
        let gap = metrics.item_gap;
        let row_height = metrics.item_height;
        for (idx, (candidate, meta)) in candidates.iter().take(count).enumerate() {
            let row = egui::Rect::from_min_size(
                egui::pos2(body.left(), body.top() + idx as f32 * (row_height + gap)),
                egui::vec2(body.width(), row_height),
            );
            let selected = idx == 0;
            paint_row(&painter, idx, candidate, meta, row, selected);
        }
    }
}

fn skin_preview_card(
    ui: &mut egui::Ui,
    key: &str,
    label: &str,
    colors: CandidatePreviewColors,
    width: f32,
    selected: bool,
) -> egui::Response {
    let palette = fluent_palette(ui);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 104.0), egui::Sense::click());
    let outer_stroke = if selected || response.hovered() {
        Stroke::new(1.5, palette.accent)
    } else {
        Stroke::new(1.0, palette.border_subtle)
    };
    ui.painter()
        .rect(rect, 10.0, palette.surface_alt, outer_stroke);
    let preview = egui::Rect::from_min_max(
        rect.min + egui::vec2(5.0, 5.0),
        egui::pos2(rect.right() - 5.0, rect.bottom() - 35.0),
    );
    ui.painter()
        .rect(preview, 5.0, colors.window, Stroke::new(1.0, colors.border));
    let marker_radius = 9.0;
    let preview_items = [("输入法", "常用"), ("输入", "全拼"), ("输入方式", "词库")];
    let row_height = ((preview.height() - 12.0) / 3.0).max(18.0);
    for (idx, (candidate, meta)) in preview_items.iter().enumerate() {
        let row = egui::Rect::from_min_size(
            egui::pos2(
                preview.left() + 6.0,
                preview.top() + 6.0 + idx as f32 * (row_height + 3.0),
            ),
            egui::vec2(preview.width() - 12.0, row_height - 3.0),
        );
        let selected = idx == 0;
        if selected {
            ui.painter().rect(
                row,
                3.0,
                colors.selected,
                Stroke::new(1.0, colors.selected_border),
            );
            ui.painter().rect(
                egui::Rect::from_min_size(row.left_top(), egui::vec2(3.0, row.height())),
                0.0,
                colors.selected_border,
                Stroke::new(0.0, Color32::TRANSPARENT),
            );
            ui.painter().circle_filled(
                egui::pos2(row.left() + 10.0, row.center().y),
                marker_radius,
                colors.selected_border,
            );
            ui.painter().text(
                egui::pos2(row.left() + 10.0, row.center().y),
                egui::Align2::CENTER_CENTER,
                format!("{}", idx + 1),
                FontId::proportional(11.0),
                colors.selected_text,
            );
        } else {
            ui.painter().rect(
                row,
                3.0,
                colors.item,
                Stroke::new(0.0, Color32::TRANSPARENT),
            );
        }
        ui.painter().text(
            egui::pos2(
                row.left() + if idx == 0 { 22.0 } else { 10.0 },
                row.top() + 3.0,
            ),
            egui::Align2::LEFT_TOP,
            candidate,
            FontId::proportional(SETTINGS_MIN_HINT_FONT.max(10.5)),
            if selected {
                colors.selected_text
            } else {
                colors.text
            },
        );
        ui.painter().text(
            egui::pos2(row.left() + 28.0, row.bottom() - 5.0),
            egui::Align2::LEFT_BOTTOM,
            *meta,
            FontId::proportional(SETTINGS_MIN_HINT_FONT.max(8.5)),
            colors.muted,
        );
    }
    let title = if key.is_empty() {
        "跟随系统推荐"
    } else {
        localized_skin_name(key, label)
    };
    ui.painter().text(
        egui::pos2(rect.left() + 9.0, rect.bottom() - 24.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(SETTINGS_FONT_SMALL.max(SETTINGS_MIN_HINT_FONT)),
        if selected {
            palette.accent
        } else {
            palette.text
        },
    );
    let english = if key.is_empty() {
        "Follow system recommendation"
    } else {
        label
    };
    if !english.is_empty() {
        ui.painter().text(
            egui::pos2(rect.left() + 9.0, rect.bottom() - 12.0),
            egui::Align2::LEFT_CENTER,
            english,
            FontId::proportional(SETTINGS_MIN_HINT_FONT),
            palette.muted,
        );
    }
    if selected {
        ui.painter().circle_filled(
            egui::pos2(rect.right() - 14.0, rect.bottom() - 17.0),
            9.0,
            palette.accent,
        );
        ui.painter().text(
            egui::pos2(rect.right() - 14.0, rect.bottom() - 17.0),
            egui::Align2::CENTER_CENTER,
            "✓",
            FontId::proportional(12.0),
            palette.accent_text,
        );
    }
    response.on_hover_text(if key.is_empty() {
        "根据系统明暗模式选择推荐外观".to_string()
    } else {
        format!("{label} · {key}")
    })
}

fn localized_skin_name<'a>(key: &str, fallback: &'a str) -> &'a str {
    match key {
        "light" => "静谧浅色",
        "dark" => "石墨深色",
        "mint-glass" => "薄荷玻璃",
        "high-visibility" => "高对比度",
        "retro-terminal" => "复古终端",
        "sea-salt" => "海盐",
        "sunlit-amber" => "日光琥珀",
        "paper-latte" => "纸张拿铁",
        "moon-ink" => "月墨",
        "nordic-frost" => "北境霜色",
        "forest-ink" => "森林墨色",
        "ink-violet" => "墨紫",
        "neon-night" => "霓虹夜色",
        "rose-gold" => "玫瑰金",
        "cherry-pop" => "樱桃汽水",
        _ => fallback,
    }
}

fn skin_is_dark(skin: &SkinPreview) -> bool {
    let color = parse_skin_color(&skin.window_bg, Color32::WHITE);
    let luminance =
        0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32;
    luminance < 128.0
}

fn skin_card_grid_rows(
    ui: &mut egui::Ui,
    model: &mut SettingsModel,
    skins: &[&SkinPreview],
    grid_id: &str,
    include_default: bool,
) {
    let palette = fluent_palette(ui);
    let available = ui.available_width();
    let columns = ((available + 8.0) / 168.0).floor().clamp(1.0, 4.0) as usize;
    let card_width =
        ((available - 8.0 * columns.saturating_sub(1) as f32) / columns as f32).clamp(132.0, 210.0);
    let default_colors = preview_colors_for_skin(palette, None);
    egui::Grid::new(grid_id)
        .num_columns(columns)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            let mut index = 0usize;
            if include_default {
                if skin_preview_card(
                    ui,
                    "",
                    "跟随系统推荐",
                    default_colors,
                    card_width,
                    model.candidate_skin_file.trim().is_empty(),
                )
                .clicked()
                {
                    model.candidate_skin_file.clear();
                }
                index += 1;
                if index % columns == 0 {
                    ui.end_row();
                }
            }
            for skin in skins {
                let colors = preview_colors_for_skin(palette, Some(skin));
                if skin_preview_card(
                    ui,
                    &skin.key,
                    &skin.display_name,
                    colors,
                    card_width,
                    skin_key_from_config(&model.candidate_skin_file)
                        .is_some_and(|key| key.eq_ignore_ascii_case(&skin.key)),
                )
                .clicked()
                {
                    model.candidate_skin_file.clone_from(&skin.key);
                }
                index += 1;
                if index % columns == 0 {
                    ui.end_row();
                }
            }
            if index % columns != 0 {
                ui.end_row();
            }
        });
}

fn skin_card_grid(ui: &mut egui::Ui, model: &mut SettingsModel, skins: &[SkinPreview]) {
    let filter_id = egui::Id::new("candidate_skin_filter");
    let mut filter = ui.memory(|memory| memory.data.get_temp::<u8>(filter_id).unwrap_or(0));
    ui.horizontal_wrapped(|ui| {
        for (value, label) in [(0, "全部"), (1, "浅色"), (2, "深色"), (3, "高对比度")] {
            if ui.selectable_label(filter == value, label).clicked() {
                filter = value;
            }
        }
    });
    ui.memory_mut(|memory| memory.data.insert_temp(filter_id, filter));
    ui.add_space(8.0);
    let visible: Vec<&SkinPreview> = skins
        .iter()
        .filter(|skin| match filter {
            1 => !skin_is_dark(skin) && skin.key != "high-visibility",
            2 => skin_is_dark(skin) && skin.key != "high-visibility",
            3 => skin.key == "high-visibility",
            _ => true,
        })
        .collect();
    skin_card_grid_rows(ui, model, &visible, "candidate_skin_cards", filter == 0);
}

pub(super) fn candidate_appearance_ui(
    ui: &mut egui::Ui,
    model: &mut SettingsModel,
    status: &mut String,
    skins: &[SkinPreview],
    chinese_fonts: &[String],
    request_real_preview: &mut bool,
) {
    section_panel(ui, "候选栏实时预览", |ui| {
        let palette = fluent_palette(ui);
        ui.label(
            RichText::new("下方预览会跟随横竖布局、字号、透明度、候选信息和皮肤设置变化。")
                .size(SETTINGS_FONT_SMALL.max(SETTINGS_MIN_HINT_FONT))
                .color(palette.muted),
        );
        ui.add_space(8.0);
        candidate_live_preview(ui, model, skins);
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if outline_button(ui, "保存并用真实候选窗试用").clicked() {
                *request_real_preview = true;
            }
            ui.label(
                RichText::new("调用与输入时相同的 C++ 渲染器，10 秒后自动关闭。")
                    .small()
                    .color(palette.muted),
            );
        });
    });

    ui.add_space(10.0);
    section_panel(ui, "推荐外观", |ui| {
        ui.horizontal_wrapped(|ui| {
            if outline_button(ui, "恢复推荐").clicked() {
                apply_candidate_recommended_defaults(model);
                *status = "候选栏已恢复推荐外观。".to_string();
            }
            if outline_button(ui, "游戏紧凑").clicked() {
                apply_candidate_game_compact(model);
                *status = "候选栏已切到游戏紧凑推荐。".to_string();
            }
            if outline_button(ui, "低延迟紧凑").clicked() {
                apply_candidate_low_latency_compact(model);
                *status = "候选栏已切到低延迟紧凑外观。".to_string();
            }
        });
        let palette = fluent_palette(ui);
        ui.label(
            RichText::new("这些按钮只调整候选栏显示，不会改热键、词库或隐私设置。")
                .small()
                .color(palette.muted),
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "布局与显示", |ui| {
        setting_slider_usize(
            ui,
            "每页候选数",
            "候选窗口单页显示的候选数量。",
            &mut model.candidate_page_size,
            3..=9,
        );
        setting_toggle(
            ui,
            "横向候选栏",
            "开启后以横排卡片显示候选。",
            &mut model.candidate_horizontal,
        );
        setting_slider_usize(
            ui,
            "横向候选数",
            "横排模式下每页显示的候选数量。",
            &mut model.candidate_horizontal_count,
            3..=9,
        );
        ui.collapsing("高级布局与交互", |ui| {
            setting_toggle(
                ui,
                "横排紧凑",
                "减少横排候选卡片间距。",
                &mut model.candidate_horizontal_compact,
            );
            setting_combo_row(
                ui,
                "外观密度",
                "统一调整字号周围的留白、行距和候选间距。",
                density_label(&model.candidate_density).to_owned(),
                "candidate_density",
                |ui| {
                    selectable_string(
                        ui,
                        &mut model.candidate_density,
                        schema_options::CANDIDATE_DENSITIES[0],
                        "紧凑",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_density,
                        schema_options::CANDIDATE_DENSITIES[1],
                        "标准",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_density,
                        schema_options::CANDIDATE_DENSITIES[2],
                        "舒适",
                    );
                },
            );
            setting_combo_row(
                ui,
                "竖排布局",
                "控制竖向候选窗口的行距、边框和选中样式。",
                vertical_layout_label(&model.candidate_vertical_layout_variant).to_owned(),
                "vertical_layout",
                |ui| {
                    selectable_string(
                        ui,
                        &mut model.candidate_vertical_layout_variant,
                        schema_options::CANDIDATE_LAYOUTS[0],
                        "经典",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_vertical_layout_variant,
                        schema_options::CANDIDATE_LAYOUTS[1],
                        "舒适",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_vertical_layout_variant,
                        schema_options::CANDIDATE_LAYOUTS[2],
                        "卡片",
                    );
                },
            );
            setting_combo_row(
                ui,
                "横排布局",
                "控制横向候选栏的间距、分块感和单行显示效果。",
                horizontal_layout_label(&model.candidate_horizontal_layout_variant).to_owned(),
                "horizontal_layout",
                |ui| {
                    selectable_string(
                        ui,
                        &mut model.candidate_horizontal_layout_variant,
                        schema_options::CANDIDATE_LAYOUTS[0],
                        "单行舒适",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_horizontal_layout_variant,
                        schema_options::CANDIDATE_LAYOUTS[1],
                        "单行紧凑",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_horizontal_layout_variant,
                        schema_options::CANDIDATE_LAYOUTS[2],
                        "分块卡片",
                    );
                },
            );
            setting_toggle(
                ui,
                "候选窗置顶",
                "候选窗口浮在宿主应用上方。",
                &mut model.candidate_topmost,
            );
            setting_toggle(
                ui,
                "候选左键提交",
                "允许在候选栏用鼠标左键提交候选。",
                &mut model.candidate_left_click,
            );
            setting_toggle(
                ui,
                "候选右键菜单",
                "允许在候选栏用鼠标右键打开固定菜单。",
                &mut model.candidate_right_click,
            );
            setting_toggle(
                ui,
                "滚轮翻页",
                "鼠标滚轮在候选窗口上切换页。",
                &mut model.paging_on_scroll,
            );
            setting_toggle(
                ui,
                "输入框内显示预编辑",
                "拼音串显示在宿主输入框内部。",
                &mut model.inline_preedit,
            );
            setting_toggle(
                ui,
                "增强候选窗定位",
                "优先跟随光标和编辑区域定位候选窗。",
                &mut model.enhanced_position,
            );
            setting_toggle(
                ui,
                "减少动态效果",
                "关闭候选窗出现、切换、悬停和翻页动画；系统关闭动画或启用高对比度时也会自动生效。",
                &mut model.candidate_reduce_motion,
            );
        });
    });

    ui.add_space(10.0);
    section_panel(ui, "字体与主题", |ui| {
        setting_slider_usize(
            ui,
            "候选字号",
            "候选正文的显示字号。",
            &mut model.candidate_font_size,
            14..=28,
        );
        ui.collapsing("高级字体", |ui| {
            setting_slider_usize(
                ui,
                "透明度",
                "候选窗整体透明度。",
                &mut model.candidate_opacity,
                90..=100,
            );
            let current_font = model.candidate_font_file.trim().to_owned();
            let default_font_label = "默认（Microsoft YaHei）";
            let uses_default_font =
                current_font.is_empty() || current_font == DEFAULT_CANDIDATE_FONT_FAMILY;
            let selected_font = if uses_default_font {
                default_font_label.to_owned()
            } else {
                current_font.clone()
            };
            setting_combo_row(
                ui,
                "中文字体",
                "候选窗口使用的字体族。",
                selected_font,
                "candidate_font_family",
                |ui| {
                    selectable_string(
                        ui,
                        &mut model.candidate_font_file,
                        DEFAULT_CANDIDATE_FONT_FAMILY,
                        default_font_label,
                    );
                    if !uses_default_font && !chinese_fonts.iter().any(|name| name == &current_font)
                    {
                        selectable_string(
                            ui,
                            &mut model.candidate_font_file,
                            &current_font,
                            &current_font,
                        );
                    }
                    for font in chinese_fonts {
                        selectable_string(ui, &mut model.candidate_font_file, font, font);
                    }
                },
            );
        });
        let palette = fluent_palette(ui);
        ui.label(
            RichText::new("皮肤")
                .strong()
                .size(SETTINGS_FONT_SETTING_TITLE)
                .color(palette.text),
        );
        ui.label(
            RichText::new("点击卡片即可切换；上方候选栏会立即预览实际配色。")
                .size(SETTINGS_FONT_SMALL.max(SETTINGS_MIN_HINT_FONT))
                .color(palette.muted),
        );
        ui.add_space(8.0);
        skin_card_grid(ui, model, skins);
        ui.add_space(10.0);
        ui.collapsing("高级主题", |ui| {
            setting_combo_row(
                ui,
                "主题",
                "跟随系统或固定浅色/深色。",
                theme_label(&model.theme).to_owned(),
                "candidate_theme",
                |ui| {
                    selectable_string(ui, &mut model.theme, schema_options::THEMES[0], "自动");
                    selectable_string(ui, &mut model.theme, schema_options::THEMES[1], "浅色");
                    selectable_string(ui, &mut model.theme, schema_options::THEMES[2], "深色");
                    selectable_string(ui, &mut model.theme, schema_options::THEMES[3], "高对比度");
                },
            );
        });
        ui.collapsing("高级材质与字重", |ui| {
            setting_combo_row(
                ui,
                "材质",
                "候选窗口背景和层次风格。",
                material_label(&model.candidate_material).to_owned(),
                "candidate_material",
                |ui| {
                    selectable_string(
                        ui,
                        &mut model.candidate_material,
                        schema_options::CANDIDATE_MATERIALS[0],
                        "自动",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_material,
                        schema_options::CANDIDATE_MATERIALS[1],
                        "实心",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_material,
                        schema_options::CANDIDATE_MATERIALS[2],
                        "渐变",
                    );
                    selectable_string(
                        ui,
                        &mut model.candidate_material,
                        schema_options::CANDIDATE_MATERIALS[3],
                        "柔雾",
                    );
                },
            );
            setting_slider_usize(
                ui,
                "普通",
                "未选中候选的字重。",
                &mut model.candidate_font_weight,
                300..=700,
            );
            setting_slider_usize(
                ui,
                "当前",
                "选中候选的字重。",
                &mut model.candidate_selected_font_weight,
                400..=800,
            );
            setting_slider_usize(
                ui,
                "标签",
                "序号标签的字重。",
                &mut model.candidate_label_font_weight,
                400..=800,
            );
            setting_slider_usize(
                ui,
                "胶囊",
                "模式标签的字重。",
                &mut model.candidate_chip_font_weight,
                350..=700,
            );
        });
    });

    ui.add_space(10.0);
    ui.collapsing("高级候选信息", |ui| {
        section_panel(ui, "候选信息与调试标记", |ui| {
            setting_toggle(
                ui,
                "显示读音 / 拼音",
                "仅在当前候选下显示读音信息。",
                &mut model.show_candidate_reading,
            );
            setting_toggle(
                ui,
                "显示候选分数",
                "调试排序时仅在当前候选显示分值。",
                &mut model.show_candidate_score,
            );
            setting_toggle(
                ui,
                "高亮显示纠错候选",
                "纠错或音近候选会在候选文本前显示 ~ 标记。",
                &mut model.highlight_typo_candidates,
            );
            setting_toggle(
                ui,
                "显示候选来源",
                "仅在当前候选显示用户、专业词库、纠错等来源。",
                &mut model.show_candidate_source,
            );
            setting_toggle(
                ui,
                "在候选栏显示模式",
                "在候选窗口顶部显示中英、标点、双拼等状态。",
                &mut model.show_mode_in_candidate_header,
            );
            setting_slider_usize(
                ui,
                "候选缩略长度",
                "过长候选在窗口中的截断长度。",
                &mut model.candidate_abbreviate_length,
                16..=256,
            );
        });
    });
}

fn apply_candidate_recommended_defaults(model: &mut SettingsModel) {
    let default = SettingsModel::default();
    model.candidate_page_size = default.candidate_page_size;
    model.candidate_horizontal = default.candidate_horizontal;
    model.candidate_horizontal_count = default.candidate_horizontal_count;
    model.candidate_horizontal_compact = default.candidate_horizontal_compact;
    model.candidate_font_size = default.candidate_font_size;
    model.candidate_opacity = default.candidate_opacity;
    model.candidate_reduce_motion = default.candidate_reduce_motion;
    model.candidate_font_weight = default.candidate_font_weight;
    model.candidate_selected_font_weight = default.candidate_selected_font_weight;
    model.candidate_label_font_weight = default.candidate_label_font_weight;
    model.candidate_chip_font_weight = default.candidate_chip_font_weight;
    model.candidate_material = default.candidate_material;
    model.candidate_density = default.candidate_density;
    model.candidate_vertical_layout_variant = default.candidate_vertical_layout_variant;
    model.candidate_horizontal_layout_variant = default.candidate_horizontal_layout_variant;
    model.candidate_topmost = default.candidate_topmost;
    model.show_candidate_reading = default.show_candidate_reading;
    model.show_candidate_score = default.show_candidate_score;
    model.highlight_typo_candidates = default.highlight_typo_candidates;
    model.show_candidate_source = default.show_candidate_source;
    model.show_mode_in_candidate_header = default.show_mode_in_candidate_header;
    model.candidate_abbreviate_length = default.candidate_abbreviate_length;
}

fn apply_candidate_game_compact(model: &mut SettingsModel) {
    model.candidate_page_size = 5;
    model.candidate_horizontal = true;
    model.candidate_horizontal_count = 5;
    model.candidate_horizontal_compact = true;
    model.candidate_material = "solid".to_string();
    model.candidate_density = "compact".to_string();
    model.candidate_horizontal_layout_variant = schema_options::CANDIDATE_LAYOUTS[1].to_string();
    model.candidate_topmost = true;
    model.show_candidate_reading = false;
    model.show_candidate_score = false;
    model.show_candidate_source = false;
    model.show_mode_in_candidate_header = false;
}

fn apply_candidate_low_latency_compact(model: &mut SettingsModel) {
    apply_candidate_game_compact(model);
    model.candidate_opacity = 100;
    model.candidate_font_weight = 500;
    model.candidate_selected_font_weight = 600;
    model.candidate_abbreviate_length = 48;
}
