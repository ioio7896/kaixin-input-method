use super::*;

const LONG_DESCRIPTION: &str = "配置、词库、剪贴板和日志所在目录。路径与说明较长时应自动换行，操作按钮应保留足够空间，下一项设置不能覆盖当前内容。";

pub(super) fn with_layout(
    width: f32,
    spacing: f32,
    scale: f32,
    mut render: impl FnMut(&mut egui::Ui),
) {
    let ctx = egui::Context::default();
    fonts::install_cjk_fonts(&ctx);
    enforce_settings_min_font_size(&ctx);
    // A second frame verifies the settled layout after font and DPI initialization.
    for _ in 0..2 {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width + 36.0, 3000.0),
            )),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(scale);
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().inner_margin(egui::Margin::same(18.0)))
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                    render(ui);
                });
        });
    }
}

fn assert_inside(rect: egui::Rect, bounds: egui::Rect) {
    assert!(
        rect.left() >= bounds.left() - 1.0,
        "left overflow: {rect:?} / {bounds:?}"
    );
    assert!(
        rect.right() <= bounds.right() + 1.0,
        "right overflow: {rect:?} / {bounds:?}"
    );
}

#[test]
fn responsive_columns_never_overlap_at_small_spacing() {
    for width in [
        240.0, 320.0, 440.0, 519.0, 520.0, 552.0, 600.0, 780.0, 792.0, 900.0,
    ] {
        for spacing in [0.0, 3.0, 9.0] {
            for scale in [1.0, 1.5] {
                with_layout(width, spacing, scale, |ui| {
                    let bounds = ui.available_rect_before_wrap();
                    let mut text_rect = egui::Rect::NOTHING;
                    let mut controls = Vec::new();
                    let row = ui
                        .scope(|ui| {
                            responsive_settings_row(
                                ui,
                                320.0,
                                |ui| {
                                    text_rect =
                                        ui.add(egui::Label::new(LONG_DESCRIPTION).wrap()).rect;
                                },
                                |ui| {
                                    let mut path = "C:/资料/开心输入法/截图".to_string();
                                    controls.push(
                                        ui.add_sized(
                                            [bounded_control_width(ui, 260.0), 24.0],
                                            TextEdit::singleline(&mut path),
                                        )
                                        .rect,
                                    );
                                    controls.push(outline_button(ui, "选择").rect);
                                    controls.push(outline_button(ui, "清除").rect);
                                },
                            );
                        })
                        .response
                        .rect;
                    assert_inside(row, bounds);
                    assert_inside(text_rect, bounds);
                    for (index, control) in controls.iter().enumerate() {
                        assert_inside(*control, bounds);
                        assert!(
                            !control.intersects(text_rect),
                            "text/control overlap: {text_rect:?} / {control:?}"
                        );
                        assert!(
                            row.contains_rect(*control),
                            "row failed to grow: {row:?} / {control:?}"
                        );
                        for other in &controls[..index] {
                            assert!(
                                !control.intersects(*other),
                                "wrapped controls overlap: {control:?} / {other:?}"
                            );
                        }
                    }
                });
            }
        }
    }
}

#[test]
fn actual_settings_rows_fit_and_reserve_wrapped_height() {
    for width in [240.0, 320.0, 440.0, 520.0, 560.0, 600.0, 792.0, 900.0] {
        for spacing in [0.0, 9.0] {
            with_layout(width, spacing, 1.5, |ui| {
                let bounds = ui.available_rect_before_wrap();
                let mut rows = Vec::new();
                let mut path = "C:/资料/开心输入法/截图/较长的保存目录".to_string();
                rows.push(
                    ui.scope(|ui| {
                        data_location_row(
                            ui,
                            "本地数据目录",
                            LONG_DESCRIPTION,
                            Path::new(&path),
                            |ui| {
                                outline_button(ui, "打开");
                            },
                        );
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        folder_path_row(
                            ui,
                            "截图保存目录",
                            LONG_DESCRIPTION,
                            &mut path,
                            "默认目录",
                        );
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        executable_path_row(
                            ui,
                            "翻译工具路径",
                            LONG_DESCRIPTION,
                            &mut path,
                            "选择程序",
                        );
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        filename_pattern_row(ui, "文件名模板", LONG_DESCRIPTION, &mut path, "模板");
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        setting_slider_usize(ui, "外观密度", LONG_DESCRIPTION, &mut 100, 50..=150);
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        setting_slider_f64(ui, "缩放比例", LONG_DESCRIPTION, &mut 1.0, 0.5..=2.0);
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        setting_toggle(ui, "全局隐私模式", LONG_DESCRIPTION, &mut false);
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        setting_combo_row(
                            ui,
                            "候选排列",
                            LONG_DESCRIPTION,
                            "竖排",
                            "layout_test",
                            |ui| {
                                ui.label("横排");
                            },
                        );
                    })
                    .response
                    .rect,
                );
                rows.push(
                    ui.scope(|ui| {
                        privacy_process_list_row(ui, "永不学习", LONG_DESCRIPTION, &mut path);
                    })
                    .response
                    .rect,
                );
                for (index, row) in rows.iter().enumerate() {
                    assert_inside(*row, bounds);
                    if index > 0 {
                        assert!(
                            row.top() >= rows[index - 1].bottom() - 1.0,
                            "rows overlap at width={width}, spacing={spacing}: {:?} / {row:?}",
                            rows[index - 1]
                        );
                    }
                }
            });
        }
    }
}

#[test]
fn compatibility_controls_have_no_overlapping_labels() {
    for width in [240.0, 320.0, 440.0, 600.0, 840.0, 1200.0] {
        for scale in [1.0, 1.5, 2.0] {
            let ctx = egui::Context::default();
            fonts::install_cjk_fonts(&ctx);
            enforce_settings_min_font_size(&ctx);
            let mut model = SettingsModel::default();
            for _ in 0..2 {
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width + 36.0, 3000.0),
                    )),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::none().inner_margin(egui::Margin::same(18.0)))
                        .show(ctx, |ui| {
                            ui.set_width(width.min(SETTINGS_CONTENT_MAX_WIDTH));
                            section_panel(ui, "游戏与全屏", |ui| {
                                compatibility_game_controls_ui(ui, &mut model);
                            });
                            ui.label("下一组设置");
                        });
                });
                fn collect_text(shape: &egui::Shape, texts: &mut Vec<(String, egui::Rect)>) {
                    match shape {
                        egui::Shape::Text(text) => texts.push((
                            text.galley.text().to_string(),
                            text.galley.rect.translate(text.pos.to_vec2()),
                        )),
                        egui::Shape::Vec(shapes) => {
                            for shape in shapes {
                                collect_text(shape, texts);
                            }
                        }
                        _ => {}
                    }
                }
                let mut texts = Vec::new();
                for clipped in &output.shapes {
                    collect_text(&clipped.shape, &mut texts);
                }
                let bounds = egui::Rect::from_min_max(
                    egui::pos2(18.0, 18.0),
                    egui::pos2(width + 18.0, 3000.0),
                );
                for (index, (label, rect)) in texts.iter().enumerate() {
                    assert!(
                        rect.right() <= bounds.right() + 1.0,
                        "text overflow at {width}/{scale}: {label} {rect:?}; texts={texts:?}"
                    );
                    assert_inside(*rect, bounds);
                    for (other_label, other) in &texts[..index] {
                        assert!(!rect.shrink(0.5).intersects(other.shrink(0.5)),
                            "overlap at {width}/{scale}: {label} {rect:?} / {other_label} {other:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn trailing_controls_share_a_column_and_align_with_title_lines() {
    for width in [480.0, 600.0, 880.0] {
        with_layout(width, 4.0, 1.5, |ui| {
            let mut positions = Vec::new();
            for kind in [0, 1, 2] {
                let mut title_rect = egui::Rect::NOTHING;
                let mut control_rect = egui::Rect::NOTHING;
                responsive_settings_row(
                    ui,
                    SETTINGS_CONTROL_WIDTH,
                    |ui| {
                        title_rect = ui
                            .add_sized(
                                [ui.available_width(), SETTINGS_ROW_HEIGHT],
                                egui::Label::new("设置名称"),
                            )
                            .rect;
                        ui.label("短说明");
                    },
                    |ui| {
                        control_rect = match kind {
                            0 => capsule_switch(ui, &mut false).rect,
                            1 => {
                                ui.add_sized(
                                    [SETTINGS_CONTROL_WIDTH, SETTINGS_ROW_HEIGHT],
                                    egui::Button::new("选项"),
                                )
                                .rect
                            }
                            _ => outline_button(ui, "打开").rect,
                        };
                    },
                );
                assert!((title_rect.center().y - control_rect.center().y).abs() <= 1.0);
                assert!(title_rect.right() < control_rect.left());
                positions.push(control_rect.left());
            }
            assert!(positions.iter().all(|x| (*x - positions[0]).abs() <= 1.0));
        });
    }
}

#[test]
fn lexicon_rows_fit_long_names_and_keep_following_rows_clear() {
    for width in [240.0, 280.0, 320.0, 440.0, 880.0] {
        for scale in [1.0, 1.5, 2.0] {
            with_layout(width, 4.0, scale, |ui| {
                let bounds = ui.available_rect_before_wrap();
                let mut previous_bottom = bounds.top();
                for title in [
                    "办公协作",
                    "地区词库 · 全国与世界行政区划名称补充",
                    "技术开发",
                ] {
                    let row = ui
                        .scope(|ui| {
                            lexicon_list_row(
                                ui,
                                title,
                                "123456 条",
                                &mut true,
                                LONG_DESCRIPTION,
                                "lexicon/zh-ext/category_office.txt",
                            );
                        })
                        .response
                        .rect;
                    assert_inside(row, bounds);
                    assert!(row.top() >= previous_bottom - 1.0);
                    previous_bottom = row.bottom();
                }
            });
        }
    }
}

#[test]
fn lexicon_bulk_selection_changes_only_the_entire_requested_group() {
    let mut states = BTreeMap::from([
        ("category_office".to_string(), false),
        ("category_shopping".to_string(), false),
        ("category_programming".to_string(), false),
    ]);
    let tags = vec![
        "category_office".to_string(),
        "category_shopping".to_string(),
        "unknown".to_string(),
    ];
    set_lexicon_group_enabled(&mut states, &tags, true);
    assert!(states["category_office"] && states["category_shopping"]);
    assert!(!states["category_programming"]);
    assert!(!states.contains_key("unknown"));
    set_lexicon_group_enabled(&mut states, &tags, false);
    assert!(states.values().all(|value| !value));
}

#[test]
fn short_descriptions_keep_wide_settings_rows_compact() {
    with_layout(880.0, 4.0, 1.5, |ui| {
        let row = ui
            .scope(|ui| {
                setting_toggle(ui, "默认英文模式", "启动时进入英文直输。", &mut false);
            })
            .response
            .rect;
        assert!(
            (28.0..=36.0).contains(&row.height()),
            "ordinary row grew: {row:?}"
        );
    });
}

#[test]
fn long_help_is_condensed_without_hiding_privacy_or_effect_conditions() {
    let ordinary = "截图完成后把实际图片文件直接交给本地文字识别工具，并立即显示预览和识别进度；后续结果可复制或发送到翻译工具。";
    assert!(setting_description_summary("自动识别", ordinary).len() < ordinary.len());
    for description in [
        "开启后强制 ASCII，不显示候选、不学习，并停止剪贴板捕获和历史读取；这些限制会持续到关闭隐私模式。",
        "仅在副本目录已配置时保存截图；保存结果会显示在状态栏，用户可打开指定目录查看文件并确认内容。",
        "修改快捷键后需要切换窗口才会生效；正在输入的宿主会继续使用原来的快捷键直到下一次重新获取焦点。",
    ] {
        assert_eq!(setting_description_summary("设置名称", description), description);
    }
}

pub(super) fn footer_test_app() -> SettingsApp {
    let model = SettingsModel::default();
    SettingsApp {
        last_saved_model: model.clone(),
        model,
        config: IniDoc::default(),
        config_path: PathBuf::from("target/unused-layout-config.ini"),
        status: String::new(),
        save_toast: None,
        confirm_close_with_unsaved_changes: false,
        user_phrase_key: String::new(),
        user_phrase_text: String::new(),
        blocked_phrase_text: String::new(),
        blocked_phrases: Vec::new(),
        blocked_phrases_loaded: false,
        user_dict_task_rx: None,
        available_skins: Vec::new(),
        available_chinese_fonts: Vec::new(),
        recent_processes: Vec::new(),
        foreground_process: None,
        game_test_wizard: None,
        compat_selected_rule: 0,
        active_section: SettingsSection::Input,
        appearance_page: AppearanceSettingsPage::Layout,
        lexicon_page: LexiconSettingsPage::Learning,
        tool_page: ToolSettingsPage::Clipboard,
        system_page: SystemSettingsPage::General,
        settings_search: String::new(),
        reset_section_scroll: false,
        vv_command_filter: String::new(),
        lexicon_filter: String::new(),
        lexicon_catalog: None,
        lexicon_catalog_rx: None,
        diagnostics_cache: None,
        diagnostics_rx: None,
        performance_export_rx: None,
        translation_check_rx: None,
    }
}

fn painted_text(shape: &egui::Shape, texts: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Text(text) => texts.push((
            text.galley.text().to_string(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                painted_text(shape, texts);
            }
        }
        _ => {}
    }
}

#[test]
fn hotkeys_page_stays_inside_window_at_all_sizes_and_scales() {
    for width in [780.0, 960.0, 1160.0, 1440.0, 1920.0] {
        for scale in [1.0, 1.5, 2.0] {
            for height in [680.0, 860.0, 3000.0] {
                let ctx = egui::Context::default();
                fonts::install_cjk_fonts(&ctx);
                enforce_settings_min_font_size(&ctx);
                let mut app = footer_test_app();
                app.active_section = SettingsSection::Hotkeys;
                let before = app.model.clone();
                for _ in 0..2 {
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    };
                    input
                        .viewports
                        .get_mut(&egui::ViewportId::ROOT)
                        .unwrap()
                        .native_pixels_per_point = Some(scale);
                    let mut bounds = egui::Rect::NOTHING;
                    let output = ctx.run(input, |ctx| {
                        egui::SidePanel::left("test_navigation")
                            .exact_width(SETTINGS_NAV_WIDTH)
                            .show(ctx, |_| {});
                        egui::CentralPanel::default()
                            .frame(egui::Frame::none())
                            .show(ctx, |ui| {
                                bounds = ui.available_rect_before_wrap();
                                settings_content_ui(ui, &mut app, &mut false);
                                assert_inside(ui.min_rect(), bounds);
                            });
                    });
                    let mut texts = Vec::new();
                    for clipped in &output.shapes {
                        painted_text(&clipped.shape, &mut texts);
                    }
                    for (index, (label, rect)) in texts.iter().enumerate() {
                        assert_inside(*rect, bounds);
                        for (other_label, other) in &texts[..index] {
                            assert!(!rect.shrink(0.5).intersects(other.shrink(0.5)),
                            "hotkeys overlap at {width}/{scale}: {label} {rect:?} / {other_label} {other:?}");
                        }
                    }
                    if height >= 3000.0 {
                        assert!(texts.iter().any(|(label, _)| label == "中英翻译快捷键"));
                    }
                    let title_left = texts
                        .iter()
                        .find(|(label, _)| label == "标点切换")
                        .unwrap()
                        .1
                        .left();
                    for title in [
                        "简繁切换",
                        "游戏中文聊天 / 兼容开关",
                        "截图快捷键",
                        "中英翻译快捷键",
                    ] {
                        let Some((_, rect)) = texts.iter().find(|(label, _)| label == title) else {
                            continue;
                        };
                        assert!(
                            (rect.left() - title_left).abs() <= 1.0,
                            "title alignment changed at {width}/{scale}: {title} {rect:?}"
                        );
                    }
                    assert!(app.model == before, "render changed hotkeys");
                }
            }
        }
    }
}

// Optional software-rendered preview from the real egui meshes and font atlas.
// It lets layout QA run on build machines without an interactive desktop.
#[test]
fn export_hotkeys_layout_preview() {
    let Ok(destination) = std::env::var("KX_HOTKEY_LAYOUT_PREVIEW") else {
        return;
    };
    let ctx = egui::Context::default();
    ctx.set_visuals(egui::Visuals::light());
    fonts::install_cjk_fonts(&ctx);
    enforce_settings_min_font_size(&ctx);
    let mut app = footer_test_app();
    app.active_section = SettingsSection::Hotkeys;
    let mut textures = Vec::new();
    for _ in 0..2 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1160.0, 1800.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("preview_navigation")
                    .exact_width(SETTINGS_NAV_WIDTH)
                    .show(ctx, |ui| {
                        fluent_nav_header(ui);
                        ui.separator();
                        ui.label("输入");
                        ui.label("外观");
                        ui.label("词库与学习");
                        ui.label(RichText::new("快捷键").strong());
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        settings_content_ui(ui, &mut app, &mut false);
                    });
            },
        );
        for (id, delta) in &output.textures_delta.set {
            let pixels: Vec<_> = match &delta.image {
                egui::ImageData::Font(image) => {
                    image.srgba_pixels(None).map(|p| p.to_array()).collect()
                }
                egui::ImageData::Color(image) => {
                    image.pixels.iter().map(|p| p.to_array()).collect()
                }
            };
            textures.push(serde_json::json!({"id": format!("{id:?}"), "size": delta.image.size(), "pos": delta.pos, "pixels": pixels}));
        }
        let meshes: Vec<_> = ctx.tessellate(output.shapes, output.pixels_per_point).into_iter().filter_map(|clipped| {
            let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive else { return None; };
            let vertices: Vec<_> = mesh.vertices.iter().map(|v| serde_json::json!([v.pos.x, v.pos.y, v.uv.x, v.uv.y, v.color.to_array()])).collect();
            Some(serde_json::json!({"clip": [clipped.clip_rect.min.x, clipped.clip_rect.min.y, clipped.clip_rect.max.x, clipped.clip_rect.max.y], "texture": format!("{:?}", mesh.texture_id), "vertices": vertices, "indices": mesh.indices}))
        }).collect();
        fs::write(&destination, serde_json::to_vec(&serde_json::json!({"width":1160, "height":1800, "textures":textures, "meshes":meshes})).unwrap()).unwrap();
    }
}

#[test]
fn footer_save_button_does_not_move_when_status_wraps_or_dirty_state_changes() {
    for width in [320.0, 440.0, 680.0, 900.0] {
        let ctx = egui::Context::default();
        fonts::install_cjk_fonts(&ctx);
        enforce_settings_min_font_size(&ctx);
        let mut app = footer_test_app();
        let mut baseline: Option<(f32, egui::Pos2)> = None;
        for (dirty, status) in [(false, ""), (false, LONG_DESCRIPTION), (true, "")] {
            app.status = status.to_string();
            for _ in 0..2 {
                let mut footer_rect = egui::Rect::NOTHING;
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width + 36.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::none().inner_margin(egui::Margin::same(18.0)))
                            .show(ctx, |ui| {
                                footer_rect = ui
                                    .scope(|ui| {
                                        settings_footer_ui(
                                            ui,
                                            &mut app,
                                            fluent_palette(ui),
                                            dirty,
                                            None,
                                        )
                                    })
                                    .response
                                    .rect;
                            });
                    },
                );
                let mut texts = Vec::new();
                for clipped in &output.shapes {
                    painted_text(&clipped.shape, &mut texts);
                }
                let save_rect = texts.iter().find(|(text, _)| text == SAVE_CN).unwrap().1;
                let bounds = egui::Rect::from_min_max(
                    egui::pos2(18.0, 18.0),
                    egui::pos2(width + 18.0, 600.0),
                );
                assert_inside(footer_rect, bounds);
                for (_, rect) in &texts {
                    assert_inside(*rect, bounds);
                }
                let current = (footer_rect.height(), save_rect.center());
                if let Some((height, center)) = baseline {
                    assert!(
                        (current.0 - height).abs() <= 1.0,
                        "footer height changed at {width}: {current:?} / {baseline:?}"
                    );
                    assert!(
                        current.1.distance(center) <= 1.0,
                        "save button moved at {width}: {current:?} / {baseline:?}"
                    );
                } else {
                    baseline = Some(current);
                }
            }
        }
    }
}

#[test]
fn compact_groups_stack_or_share_columns_without_overflow() {
    for width in [320.0, 480.0, 899.0, 900.0, 1200.0] {
        for scale in [1.0, 1.5, 2.0] {
            with_layout(width, 4.0, scale, |ui| {
                let bounds = ui.available_rect_before_wrap();
                let mut panes = [egui::Rect::NOTHING; 2];
                responsive_settings_columns(
                    ui,
                    "compact_group_test",
                    &mut panes,
                    |ui, panes| {
                        panes[0] = ui
                            .scope(|ui| {
                                section_panel(ui, "布局", |ui| {
                                    setting_toggle(
                                        ui,
                                        "输入框内显示预编辑",
                                        LONG_DESCRIPTION,
                                        &mut false,
                                    );
                                    setting_slider_usize(
                                        ui,
                                        "每页候选数",
                                        "候选数量。",
                                        &mut 5,
                                        3..=9,
                                    );
                                })
                            })
                            .response
                            .rect;
                    },
                    |ui, panes| {
                        panes[1] = ui
                            .scope(|ui| {
                                section_panel(ui, "交互", |ui| {
                                    setting_toggle(ui, "滚轮翻页", "切换候选页。", &mut false);
                                })
                            })
                            .response
                            .rect;
                    },
                );
                for pane in panes {
                    assert_inside(pane, bounds);
                }
                if width >= 900.0 {
                    assert!(panes[0].right() <= panes[1].left() + 1.0);
                    assert!((panes[0].top() - panes[1].top()).abs() <= 1.0);
                } else {
                    assert!(panes[0].bottom() <= panes[1].top() + 1.0);
                }
                let next = ui.label("下一组").rect;
                assert!(next.top() >= panes.iter().map(|r| r.bottom()).fold(0.0, f32::max));
            });
        }
    }
}

#[test]
fn compact_appearance_pages_fit_and_do_not_change_settings_on_render() {
    for width in [480.0, 720.0, 920.0, 1200.0] {
        for scale in [1.0, 1.5, 2.0] {
            for page in [
                AppearanceSettingsPage::Layout,
                AppearanceSettingsPage::Theme,
                AppearanceSettingsPage::Advanced,
            ] {
                let mut app = footer_test_app();
                app.active_section = SettingsSection::Appearance;
                app.appearance_page = page;
                let original = app.model.clone();
                with_layout(width, 4.0, scale, |ui| {
                    let bounds = ui.available_rect_before_wrap();
                    let mut preview = false;
                    let rect = ui
                        .scope(|ui| {
                            section_header(ui, &mut app);
                            settings_search_ui(ui, &mut app, true);
                            appearance_page_tabs(ui, &mut app);
                            candidate_page_ui(ui, &mut app, &mut preview);
                        })
                        .response
                        .rect;
                    assert_inside(rect, bounds);
                    assert!(!preview);
                    assert!(original == app.model, "render changed settings on {page:?}");
                });
            }
        }
    }
}

#[test]
fn compact_navigation_titles_fit_the_narrow_rail() {
    let ctx = egui::Context::default();
    fonts::install_cjk_fonts(&ctx);
    enforce_settings_min_font_size(&ctx);
    for _ in 0..2 {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::SidePanel::left("test_compact_rail")
                .exact_width(SETTINGS_NAV_WIDTH)
                .frame(egui::Frame::none().inner_margin(egui::Margin::same(10.0)))
                .show(ctx, |ui| {
                    fluent_nav_header(ui);
                    let model = SettingsModel::default();
                    let mut active = SettingsSection::Input;
                    for section in SettingsSection::ALL {
                        nav_item(ui, &mut active, section, &model);
                    }
                });
        });
        let mut texts = Vec::new();
        for shape in output.shapes {
            painted_text(&shape.shape, &mut texts);
        }
        for section in SettingsSection::ALL {
            let rect = texts
                .iter()
                .find(|(text, _)| text == section.label())
                .unwrap()
                .1;
            assert!(
                rect.right() <= SETTINGS_NAV_WIDTH - 10.0,
                "rail title clipped: {}",
                section.label()
            );
        }
    }
}

#[test]
fn search_opens_the_matching_subpage_without_modifying_settings() {
    let cases = [
        ("候选字体", Some(AppearanceSettingsPage::Theme), None),
        (
            "候选来源与调试标记",
            Some(AppearanceSettingsPage::Advanced),
            None,
        ),
        ("用户词库", None, Some(LexiconSettingsPage::User)),
        ("学习策略", None, Some(LexiconSettingsPage::Learning)),
        ("扩展词库", None, Some(LexiconSettingsPage::Extensions)),
    ];
    let mut app = footer_test_app();
    let original = app.model.clone();
    for (title, appearance, lexicon) in cases {
        let entry = SETTINGS_SEARCH_ENTRIES
            .iter()
            .find(|entry| entry.title == title)
            .unwrap();
        app.reset_section_scroll = false;
        app.settings_search = title.to_owned();
        navigate_to_search_entry(&mut app, entry);
        assert!(app.active_section == entry.section);
        if let Some(page) = appearance {
            assert_eq!(app.appearance_page, page);
        }
        if let Some(page) = lexicon {
            assert_eq!(app.lexicon_page, page);
        }
        assert!(app.reset_section_scroll && app.settings_search.is_empty());
        assert!(app.model == original);
    }
}

#[test]
fn compact_lexicon_and_advanced_pages_preserve_controls_and_fit() {
    for width in [320.0, 480.0, 920.0] {
        for scale in [1.0, 1.5, 2.0] {
            let mut app = footer_test_app();
            app.blocked_phrases_loaded = true;
            app.lexicon_catalog = Some(Vec::new());
            let original = app.model.clone();
            with_layout(width, 4.0, scale, |ui| {
                let bounds = ui.available_rect_before_wrap();
                for page in [
                    LexiconSettingsPage::Learning,
                    LexiconSettingsPage::User,
                    LexiconSettingsPage::Extensions,
                ] {
                    app.lexicon_page = page;
                    let rect = ui.scope(|ui| lexicon_page_ui(ui, &mut app)).response.rect;
                    assert_inside(rect, bounds);
                }
                let rect = ui.scope(|ui| advanced_page_ui(ui, &mut app)).response.rect;
                assert_inside(rect, bounds);
                assert!(original == app.model);
                assert!(app.user_dict_task_rx.is_none());
            });
        }
    }
}
