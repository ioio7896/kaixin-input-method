use super::*;

const LONG_DESCRIPTION: &str = "配置、词库、剪贴板和日志所在目录。路径与说明较长时应自动换行，操作按钮应保留足够空间，下一项设置不能覆盖当前内容。";

fn with_layout(width: f32, spacing: f32, scale: f32, mut render: impl FnMut(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    fonts::install_cjk_fonts(&ctx);
    enforce_settings_min_font_size(&ctx);
    ctx.set_pixels_per_point(scale);
    // A second frame verifies the settled layout after font and DPI initialization.
    for _ in 0..2 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width + 36.0, 3000.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().inner_margin(egui::Margin::same(18.0)))
                    .show(ctx, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                        render(ui);
                    });
            },
        );
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
                                false,
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
