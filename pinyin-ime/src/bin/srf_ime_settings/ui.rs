use super::*;

const SETTINGS_FONT_HEADING: f32 = 20.0;
const SETTINGS_FONT_PAGE_TITLE: f32 = 18.0;
const SETTINGS_FONT_BRAND_TITLE: f32 = 17.0;
const SETTINGS_FONT_SECTION_TITLE: f32 = 16.0;
const SETTINGS_FONT_SETTING_TITLE: f32 = 15.0;
const SETTINGS_FONT_NAV_TITLE: f32 = 15.0;
const SETTINGS_FONT_BODY: f32 = 15.0;
const SETTINGS_MIN_HINT_FONT: f32 = 10.0;
const SETTINGS_FONT_SMALL: f32 = 14.0;
const SETTINGS_FONT_MONOSPACE: f32 = 14.0;
const SETTINGS_FONT_LOG: f32 = 12.0;
const SETTINGS_CONTROL_WIDTH: f32 = 420.0;
const SETTINGS_CONTROL_MIN_WIDTH: f32 = 220.0;
const SETTINGS_ROW_STACK_WIDTH: f32 = 640.0;
const SETTINGS_BUTTON_WIDTH: f32 = 82.0;
const DEFAULT_SCREENSHOT_DIR_DESCRIPTION: &str = "留空时使用“图片\\Kaixin Screenshots”。";
const DEFAULT_SCREENSHOT_DIR_HINT: &str = "默认：图片\\Kaixin Screenshots";

fn screenshot_capture_scheme_label(mode: &str) -> &'static str {
    if mode == "current_window" {
        "当前窗口"
    } else {
        "智能框选"
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_user_dict_task();
        if self.user_dict_task_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let palette = FluentPalette::from_visuals(&ctx.style().visuals);
        // `SettingsModel` is already `PartialEq`; comparing it avoids cloning
        // and rendering the whole INI file on every egui repaint.  Rendering
        // is deliberately kept on the save path, where it belongs.
        let is_dirty = self.model != self.last_saved_model;
        let mut request_real_candidate_preview = false;
        if ctx.input(|input| input.viewport().close_requested()) && is_dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close_with_unsaved_changes = true;
        }
        if self.confirm_close_with_unsaved_changes {
            let dialog_bounds = ctx.screen_rect().shrink(12.0);
            egui::Window::new("未保存的更改")
                .collapsible(false)
                .resizable(false)
                .constrain_to(dialog_bounds)
                .max_width(dialog_bounds.width())
                .max_height(dialog_bounds.height())
                .scroll([false, true])
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("设置尚未保存。关闭前要保存吗？");
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("保存并关闭").clicked() {
                            if self.save().is_ok() {
                                self.confirm_close_with_unsaved_changes = false;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button("放弃更改").clicked() {
                            self.last_saved_model = self.model.clone();
                            self.confirm_close_with_unsaved_changes = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("继续编辑").clicked() {
                            self.confirm_close_with_unsaved_changes = false;
                        }
                    });
                });
        }
        game_test_wizard_ui(ctx, self);
        egui::TopBottomPanel::bottom("settings_actions")
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(palette.command_bar)
                    .stroke(Stroke::new(1.0, palette.border_subtle))
                    .inner_margin(egui::Margin::symmetric(22.0, 8.0)),
            )
            .show(ctx, |ui| {
                let conflicts = hotkey_conflicts(&self.model);
                if ui.available_width() < 680.0 {
                    if !conflicts.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("快捷键冲突：{}", conflicts.join(" / ")))
                                    .small()
                                    .color(palette.warning),
                            )
                            .wrap(),
                        );
                    } else if !self.status.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.status).small().color(palette.muted),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        settings_action_buttons(ui, self, palette, is_dirty);
                    });
                } else {
                    ui.horizontal(|ui| {
                    let total_width = ui.available_width();
                    let spacing = ui.spacing().item_spacing.x;
                    let actions_width = (total_width * 0.42)
                        .clamp(320.0, 460.0)
                        .min((total_width - 140.0).max(180.0));
                    let status_width = (total_width - actions_width - spacing).max(120.0);

                    ui.allocate_ui_with_layout(
                        egui::vec2(status_width, 42.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(status_width);
                        if !conflicts.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!(
                                        "快捷键冲突：{}",
                                        conflicts.join(" / ")
                                    ))
                                    .small()
                                    .color(palette.warning),
                                )
                                .wrap(),
                            );
                        }

                        if !self.status.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.status).small().color(palette.muted),
                                )
                                .wrap(),
                            );
                        } else if conflicts.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new("设置保存在本机配置文件中；多数设置热加载，热键/兼容规则可能需要切换一次焦点。")
                                        .small()
                                        .color(palette.muted),
                                )
                                .wrap(),
                            );
                        }
                        },
                    );

                    ui.allocate_ui_with_layout(
                        egui::vec2(actions_width, 42.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.set_width(actions_width);
                        settings_action_buttons(ui, self, palette, is_dirty);
                        },
                    );
                    });
                }
            });

        let compact_navigation = ctx.available_rect().width() < SETTINGS_MIN_WINDOW_SIZE[0];
        if compact_navigation {
            egui::TopBottomPanel::top("settings_compact_nav")
                .resizable(false)
                .frame(
                    egui::Frame::none()
                        .fill(palette.nav_bg)
                        .stroke(Stroke::new(1.0, palette.border_subtle))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
                )
                .show(ctx, |ui| {
                    let previous_section = self.active_section;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(TITLE_CN).strong().color(palette.text));
                        ComboBox::from_id_salt("settings_compact_section")
                            .selected_text(self.active_section.label())
                            .width(ui.available_width().min(320.0).max(1.0))
                            .show_ui(ui, |ui| {
                                for section in SettingsSection::ALL {
                                    ui.selectable_value(
                                        &mut self.active_section,
                                        section,
                                        section.label(),
                                    );
                                }
                            });
                    });
                    if self.active_section != previous_section {
                        self.reset_section_scroll = true;
                    }
                });
        } else {
            egui::SidePanel::left("settings_nav")
                .resizable(false)
                .exact_width(SETTINGS_NAV_WIDTH)
                .frame(
                    egui::Frame::none()
                        .fill(palette.nav_bg)
                        .stroke(Stroke::new(1.0, palette.border_subtle))
                        .inner_margin(egui::Margin::symmetric(14.0, 12.0)),
                )
                .show(ctx, |ui| {
                    fluent_nav_header(ui);
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .id_salt("settings_navigation_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let previous_section = self.active_section;
                            let mut current_group = "";
                            for section in SettingsSection::ALL {
                                if section.nav_group() != current_group {
                                    if !current_group.is_empty() {
                                        ui.add_space(8.0);
                                    }
                                    current_group = section.nav_group();
                                    ui.label(
                                        RichText::new(current_group)
                                            .size(SETTINGS_FONT_SMALL)
                                            .color(palette.muted),
                                    );
                                    ui.add_space(3.0);
                                }
                                nav_item(ui, &mut self.active_section, section, &self.model);
                                ui.add_space(4.0);
                            }
                            if self.active_section != previous_section {
                                self.reset_section_scroll = true;
                            }
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("数据只保存在本机")
                                    .small()
                                    .color(palette.muted),
                            );
                        });
                });
        }

        if self.active_section == SettingsSection::Diagnostics {
            let needs_refresh = self
                .diagnostics_cache
                .as_ref()
                .map(|snapshot| snapshot.refreshed_at.elapsed() >= Duration::from_secs(2))
                .unwrap_or(true);
            if needs_refresh {
                self.diagnostics_cache = Some(build_diagnostics_snapshot(self));
            }
            // Diagnostics are intentionally sampled, not rebuilt for every
            // mouse move or scroll event.  This keeps the page responsive
            // while still showing fresh runtime data.
            ctx.request_repaint_after(Duration::from_secs(2));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(palette.app_bg))
            .show(ctx, |ui| {
                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(
                        if ui.available_width() < 480.0 {
                            12.0
                        } else {
                            24.0
                        },
                        18.0,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        section_header(ui, self.active_section);
                        ui.add_space(14.0);

                        let reset_scroll = self.reset_section_scroll;
                        self.reset_section_scroll = false;
                        let mut scroll_area = egui::ScrollArea::vertical()
                            .id_salt("settings_section_content")
                            .auto_shrink([false, false]);
                        if reset_scroll {
                            scroll_area = scroll_area.vertical_scroll_offset(0.0);
                        }
                        scroll_area.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            match self.active_section {
                                SettingsSection::Appearance => {
                                    candidate_page_ui(ui, self, &mut request_real_candidate_preview)
                                }
                                SettingsSection::Lexicon => lexicon_page_ui(ui, self),
                                SettingsSection::Hotkeys => hotkeys_ui(ui, &mut self.model),
                                SettingsSection::Clipboard => clipboard_settings_ui(ui, self),
                                SettingsSection::Compatibility => compatibility_ui(ui, self),
                                SettingsSection::Screenshot => screenshot_page_ui(ui, self),
                                SettingsSection::Ocr => ocr_page_ui(ui, self),
                                SettingsSection::Translation => translation_page_ui(ui, self),
                                SettingsSection::Privacy => privacy_data_ui(ui, self),
                                SettingsSection::Diagnostics => diagnostics_page_ui(ui, self),
                                SettingsSection::Advanced => advanced_page_ui(ui, self),
                            }
                        });
                    });
            });
        if request_real_candidate_preview && self.save().is_ok() {
            self.status = match launch_real_candidate_preview() {
                Ok(()) => "已保存设置并弹出真实候选窗；预览将在 10 秒后关闭。".to_string(),
                Err(error) => format!("无法启动真实候选窗预览：{error}"),
            };
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct FluentPalette {
    pub(super) app_bg: Color32,
    pub(super) nav_bg: Color32,
    pub(super) command_bar: Color32,
    pub(super) surface: Color32,
    pub(super) surface_alt: Color32,
    pub(super) control_bg: Color32,
    pub(super) control_hover: Color32,
    pub(super) nav_hover: Color32,
    pub(super) nav_selected: Color32,
    pub(super) border: Color32,
    pub(super) border_subtle: Color32,
    pub(super) text: Color32,
    pub(super) muted: Color32,
    pub(super) accent: Color32,
    pub(super) accent_hover: Color32,
    pub(super) accent_pressed: Color32,
    pub(super) accent_text: Color32,
    pub(super) warning: Color32,
    pub(super) success: Color32,
    pub(super) danger: Color32,
    pub(super) warning_bg: Color32,
    pub(super) success_bg: Color32,
    pub(super) danger_bg: Color32,
    pub(super) info_bg: Color32,
}

impl FluentPalette {
    fn from_visuals(visuals: &egui::Visuals) -> Self {
        if visuals.dark_mode {
            Self {
                app_bg: Color32::from_rgb(32, 32, 32),
                nav_bg: Color32::from_rgb(28, 28, 28),
                command_bar: Color32::from_rgb(36, 36, 36),
                surface: Color32::from_rgb(43, 43, 43),
                surface_alt: Color32::from_rgb(37, 37, 37),
                control_bg: Color32::from_rgb(49, 49, 49),
                control_hover: Color32::from_rgb(56, 56, 56),
                nav_hover: Color32::from_rgb(45, 45, 45),
                nav_selected: Color32::from_rgb(37, 54, 70),
                border: Color32::from_rgb(64, 64, 64),
                border_subtle: Color32::from_rgb(52, 52, 52),
                text: Color32::from_rgb(245, 245, 245),
                muted: Color32::from_rgb(200, 200, 200),
                accent: Color32::from_rgb(76, 209, 151),
                accent_hover: Color32::from_rgb(111, 224, 176),
                accent_pressed: Color32::from_rgb(47, 180, 126),
                accent_text: Color32::from_rgb(0, 0, 0),
                warning: Color32::from_rgb(255, 202, 109),
                success: Color32::from_rgb(76, 209, 151),
                danger: Color32::from_rgb(255, 112, 112),
                warning_bg: Color32::from_rgb(66, 52, 30),
                success_bg: Color32::from_rgb(28, 58, 48),
                danger_bg: Color32::from_rgb(66, 34, 34),
                info_bg: Color32::from_rgb(38, 48, 58),
            }
        } else {
            Self {
                app_bg: Color32::from_rgb(241, 244, 246),
                nav_bg: Color32::from_rgb(235, 239, 242),
                command_bar: Color32::from_rgb(246, 248, 250),
                surface: Color32::from_rgb(252, 253, 253),
                surface_alt: Color32::from_rgb(240, 245, 245),
                control_bg: Color32::from_rgb(252, 253, 254),
                control_hover: Color32::from_rgb(242, 248, 247),
                nav_hover: Color32::from_rgb(224, 233, 234),
                nav_selected: Color32::from_rgb(218, 238, 234),
                border: Color32::from_rgb(205, 213, 223),
                border_subtle: Color32::from_rgb(218, 225, 234),
                text: Color32::from_rgb(31, 41, 55),
                muted: Color32::from_rgb(96, 110, 128),
                accent: Color32::from_rgb(0, 137, 110),
                accent_hover: Color32::from_rgb(0, 120, 96),
                accent_pressed: Color32::from_rgb(0, 96, 79),
                accent_text: Color32::WHITE,
                warning: Color32::from_rgb(157, 93, 0),
                success: Color32::from_rgb(0, 137, 110),
                danger: Color32::from_rgb(190, 45, 45),
                warning_bg: Color32::from_rgb(255, 246, 224),
                success_bg: Color32::from_rgb(226, 246, 240),
                danger_bg: Color32::from_rgb(255, 235, 235),
                info_bg: Color32::from_rgb(232, 241, 248),
            }
        }
    }
}

pub(super) fn fluent_palette(ui: &egui::Ui) -> FluentPalette {
    FluentPalette::from_visuals(ui.visuals())
}

fn fluent_primary_button<'a>(label: &'a str, palette: FluentPalette) -> egui::Button<'a> {
    egui::Button::new(RichText::new(label).strong().color(palette.accent_text))
        .fill(palette.accent)
        .stroke(Stroke::new(1.0, palette.accent))
        .rounding(4.0)
        .min_size(egui::vec2(104.0, 30.0))
}

fn settings_action_buttons(
    ui: &mut egui::Ui,
    app: &mut SettingsApp,
    palette: FluentPalette,
    is_dirty: bool,
) {
    if ui.add(fluent_primary_button(SAVE_CN, palette)).clicked() {
        let _ = app.save();
    }
    if is_dirty {
        status_badge(ui, StatusTone::Warning, "未保存");
    }
    if outline_button(ui, OPEN_CFG_CN).clicked() {
        app.open_config_dir();
    }
    if danger_button(ui, RESET_CN).clicked() {
        app.reset_defaults();
    }
}

fn fluent_nav_header(ui: &mut egui::Ui) {
    let palette = fluent_palette(ui);
    egui::Frame::none()
        .fill(palette.surface_alt)
        .stroke(Stroke::new(1.0, palette.border_subtle))
        .rounding(SETTINGS_PANEL_RADIUS)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(TITLE_CN)
                    .strong()
                    .size(SETTINGS_FONT_BRAND_TITLE)
                    .color(palette.text),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new("本地、干净、可控")
                    .small()
                    .color(palette.muted),
            );
        });
}

fn section_header(ui: &mut egui::Ui, section: SettingsSection) {
    let palette = fluent_palette(ui);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(section.label())
                    .strong()
                    .size(SETTINGS_FONT_PAGE_TITLE)
                    .color(palette.text),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(section.hint())
                    .size(SETTINGS_FONT_SMALL)
                    .color(palette.muted),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            ui.label(
                RichText::new("本机配置")
                    .size(SETTINGS_FONT_SMALL)
                    .color(palette.muted),
            );
        });
    });
}

pub(super) fn enforce_settings_min_font_size(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::proportional(SETTINGS_FONT_HEADING),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(SETTINGS_FONT_BODY));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(SETTINGS_FONT_BODY));
    style.text_styles.insert(
        TextStyle::Small,
        FontId::proportional(SETTINGS_FONT_SMALL.max(SETTINGS_MIN_HINT_FONT)),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::monospace(SETTINGS_FONT_MONOSPACE),
    );
    style.spacing.item_spacing = egui::vec2(9.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);

    let palette = FluentPalette::from_visuals(&style.visuals);
    let radius = egui::Rounding::same(4.0);
    style.visuals.window_fill = palette.app_bg;
    style.visuals.panel_fill = palette.app_bg;
    style.visuals.extreme_bg_color = palette.control_bg;
    style.visuals.faint_bg_color = palette.surface_alt;
    style.visuals.widgets.noninteractive.fg_stroke.color = palette.text;
    style.visuals.widgets.inactive.fg_stroke.color = palette.text;
    style.visuals.widgets.hovered.fg_stroke.color = palette.text;
    style.visuals.widgets.active.fg_stroke.color = palette.text;
    style.visuals.widgets.open.fg_stroke.color = palette.text;
    style.visuals.widgets.inactive.bg_fill = palette.control_bg;
    style.visuals.widgets.hovered.bg_fill = palette.control_hover;
    style.visuals.widgets.active.bg_fill = palette.control_hover;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.border);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent_hover);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.accent_pressed);
    style.visuals.widgets.inactive.rounding = radius;
    style.visuals.widgets.hovered.rounding = radius;
    style.visuals.widgets.active.rounding = radius;
    style.visuals.widgets.open.rounding = radius;
    style.visuals.selection.bg_fill = palette.accent;
    style.visuals.selection.stroke.color = palette.accent_text;
    style.visuals.hyperlink_color = palette.accent;
    ctx.set_style(style);
}

fn nav_item(
    ui: &mut egui::Ui,
    current: &mut SettingsSection,
    section: SettingsSection,
    model: &SettingsModel,
) {
    let palette = fluent_palette(ui);
    let selected = *current == section;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
    let response = response.on_hover_text(section.hint());
    let fill = if selected {
        palette.nav_selected
    } else if response.hovered() {
        palette.nav_hover
    } else {
        Color32::TRANSPARENT
    };
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            egui::Rounding::same(6.0),
            fill,
            Stroke::new(0.0, Color32::TRANSPARENT),
        );
        if selected {
            let indicator = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 3.0, rect.center().y - 11.0),
                egui::vec2(4.0, 22.0),
            );
            ui.painter()
                .rect_filled(indicator, egui::Rounding::same(2.0), palette.accent);
        }
        let text_color = if selected {
            palette.accent
        } else {
            palette.text
        };
        paint_settings_icon(
            ui,
            egui::pos2(rect.left() + 38.0, rect.center().y),
            section.icon(),
            text_color,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 64.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            section.label(),
            FontId::proportional(SETTINGS_FONT_NAV_TITLE),
            text_color,
        );
        if let Some(summary) = nav_status_summary(section, model) {
            ui.painter().text(
                egui::pos2(rect.right() - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                summary,
                FontId::proportional(SETTINGS_FONT_SMALL),
                if selected {
                    palette.accent
                } else {
                    palette.muted
                },
            );
        }
    }
    if response.clicked() {
        *current = section;
    }
}

fn nav_status_summary(section: SettingsSection, model: &SettingsModel) -> Option<String> {
    match section {
        SettingsSection::Hotkeys => {
            let conflicts = hotkey_conflicts(model).len();
            (conflicts > 0).then(|| format!("{conflicts} 冲突"))
        }
        SettingsSection::Clipboard => Some(if model.clipboard_background_enabled {
            format!("{} 条", model.clipboard_max_history_items)
        } else {
            "已关闭".to_string()
        }),
        // "词库与个性化"本身较长；在窄侧栏同一行追加数量会挤压标题。
        // 词库统计保留在页面内，导航只显示不会破坏布局的状态摘要。
        SettingsSection::Lexicon => None,
        SettingsSection::Compatibility => {
            let count = model.compat_rules.len();
            (count > 0).then(|| format!("{count} 规则"))
        }
        SettingsSection::Privacy => Some(if model.privacy_enabled {
            "隐私模式".to_string()
        } else {
            "标准".to_string()
        }),
        _ => None,
    }
}

fn section_panel(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let palette = fluent_palette(ui);
    ui.add_space(2.0);
    let available_width = ui.available_width();
    egui::Frame::none()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border_subtle))
        .rounding(SETTINGS_PANEL_RADIUS)
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.set_width((available_width - 28.0).max(160.0));
            ui.label(
                RichText::new(title)
                    .strong()
                    .size(SETTINGS_FONT_SECTION_TITLE)
                    .color(palette.text),
            );
            ui.add_space(10.0);
            add_contents(ui);
            ui.add_space(4.0);
        });
    ui.add_space(10.0);
}

fn quiet_section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let palette = fluent_palette(ui);
    ui.label(
        RichText::new(title)
            .strong()
            .size(SETTINGS_FONT_SECTION_TITLE)
            .color(palette.text),
    );
    ui.add_space(8.0);
    add_contents(ui);
    ui.add_space(20.0);
}

fn paint_settings_icon(ui: &egui::Ui, center: egui::Pos2, icon: SettingsIcon, color: Color32) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.6, color);
    let line = |from: egui::Pos2, to: egui::Pos2| {
        painter.line_segment([from, to], stroke);
    };
    let rounded_rect = |rect: egui::Rect| {
        painter.rect(
            rect,
            egui::Rounding::same(2.5),
            Color32::TRANSPARENT,
            stroke,
        );
    };

    match icon {
        SettingsIcon::Appearance => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(16.0, 18.0)));
            line(
                center + egui::vec2(-5.0, -4.0),
                center + egui::vec2(5.0, -4.0),
            );
            line(
                center + egui::vec2(-5.0, 0.0),
                center + egui::vec2(5.0, 0.0),
            );
            line(
                center + egui::vec2(-5.0, 4.0),
                center + egui::vec2(2.0, 4.0),
            );
        }
        SettingsIcon::Lexicon => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(17.0, 18.0)));
            line(
                center + egui::vec2(-5.0, -4.5),
                center + egui::vec2(5.0, -4.5),
            );
            line(
                center + egui::vec2(-5.0, 0.0),
                center + egui::vec2(5.0, 0.0),
            );
            line(
                center + egui::vec2(-5.0, 4.5),
                center + egui::vec2(1.0, 4.5),
            );
        }
        SettingsIcon::Hotkeys => {
            rounded_rect(egui::Rect::from_center_size(
                center + egui::vec2(-3.5, 1.5),
                egui::vec2(10.0, 10.0),
            ));
            rounded_rect(egui::Rect::from_center_size(
                center + egui::vec2(4.0, -3.0),
                egui::vec2(7.0, 7.0),
            ));
            line(
                center + egui::vec2(-6.0, 1.5),
                center + egui::vec2(-1.0, 1.5),
            );
            line(
                center + egui::vec2(-3.5, -1.0),
                center + egui::vec2(-3.5, 4.0),
            );
        }
        SettingsIcon::Clipboard => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(18.0, 16.0)));
            line(
                center + egui::vec2(-5.0, -3.0),
                center + egui::vec2(5.0, -3.0),
            );
            line(
                center + egui::vec2(-5.0, 1.0),
                center + egui::vec2(5.0, 1.0),
            );
            line(
                center + egui::vec2(-3.0, 5.0),
                center + egui::vec2(3.0, 5.0),
            );
            rounded_rect(egui::Rect::from_center_size(
                center + egui::vec2(0.0, -8.0),
                egui::vec2(7.0, 3.0),
            ));
        }
        SettingsIcon::Compatibility => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(18.0, 14.0)));
            line(
                center + egui::vec2(-8.0, -2.5),
                center + egui::vec2(8.0, -2.5),
            );
            painter.circle_filled(center + egui::vec2(-5.5, -5.0), 0.9, color);
            painter.circle_filled(center + egui::vec2(-2.5, -5.0), 0.9, color);
            line(
                center + egui::vec2(-5.0, 2.0),
                center + egui::vec2(5.0, 2.0),
            );
            line(
                center + egui::vec2(-5.0, 5.0),
                center + egui::vec2(2.0, 5.0),
            );
        }
        SettingsIcon::Screenshot => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(17.0, 17.0)));
            painter.add(egui::Shape::line(
                vec![
                    center + egui::vec2(-7.0, 5.0),
                    center + egui::vec2(-2.0, 0.0),
                    center + egui::vec2(1.0, 3.0),
                    center + egui::vec2(4.0, -1.0),
                    center + egui::vec2(7.0, 5.0),
                ],
                stroke,
            ));
            painter.circle_filled(center + egui::vec2(4.5, -5.0), 1.3, color);
        }
        SettingsIcon::Ocr => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(17.0, 17.0)));
            painter.circle_stroke(center + egui::vec2(-1.5, -1.5), 4.5, stroke);
            line(center + egui::vec2(2.0, 2.0), center + egui::vec2(6.5, 6.5));
        }
        SettingsIcon::Translation => {
            painter.circle_stroke(center, 8.0, stroke);
            line(
                center + egui::vec2(-8.0, 0.0),
                center + egui::vec2(8.0, 0.0),
            );
            line(
                center + egui::vec2(0.0, -8.0),
                center + egui::vec2(0.0, 8.0),
            );
            line(
                center + egui::vec2(-4.0, -5.0),
                center + egui::vec2(-4.0, 5.0),
            );
        }
        SettingsIcon::Privacy => {
            let points = vec![
                center + egui::vec2(0.0, -9.0),
                center + egui::vec2(7.0, -6.0),
                center + egui::vec2(6.0, 2.0),
                center + egui::vec2(0.0, 9.0),
                center + egui::vec2(-6.0, 2.0),
                center + egui::vec2(-7.0, -6.0),
            ];
            painter.add(egui::Shape::closed_line(points, stroke));
            line(
                center + egui::vec2(-3.0, 0.0),
                center + egui::vec2(-0.5, 2.5),
            );
            line(
                center + egui::vec2(-0.5, 2.5),
                center + egui::vec2(4.0, -2.5),
            );
        }
        SettingsIcon::Diagnostics => {
            for (y, knob_x) in [(-6.0, -2.0), (0.0, 4.0), (6.0, -4.0)] {
                line(center + egui::vec2(-8.0, y), center + egui::vec2(8.0, y));
                painter.circle_filled(center + egui::vec2(knob_x, y), 2.0, color);
            }
        }
        SettingsIcon::Advanced => {
            painter.circle_stroke(center, 7.0, stroke);
            painter.circle_filled(center, 2.0, color);
            for angle in [0.0_f32, 1.57, 3.14, 4.71] {
                let direction = egui::vec2(angle.cos(), angle.sin());
                line(center + direction * 7.0, center + direction * 9.0);
            }
        }
    }
}

fn tool_symbol(ui: &mut egui::Ui, symbol: &str, fill: Color32) {
    let palette = fluent_palette(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
    ui.painter()
        .rect(rect, 4.0, fill, Stroke::new(1.0, palette.border_subtle));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        symbol,
        FontId::proportional(SETTINGS_FONT_SMALL),
        palette.text,
    );
}

fn tool_row(
    ui: &mut egui::Ui,
    symbol: &str,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let palette = fluent_palette(ui);
    let row = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(0.0, 7.0))
        .show(ui, |ui| {
            let total_width = ui.available_width();
            if total_width < SETTINGS_ROW_STACK_WIDTH {
                ui.vertical(|ui| {
                    ui.set_width(total_width);
                    ui.horizontal(|ui| {
                        tool_symbol(ui, symbol, palette.surface_alt);
                        ui.vertical(|ui| {
                            ui.set_width((total_width - 42.0).max(160.0));
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(description)
                                        .size(SETTINGS_FONT_SMALL)
                                        .color(palette.muted),
                                )
                                .wrap(),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(total_width, 34.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(total_width);
                            add_control(ui);
                        },
                    );
                });
            } else {
                let spacing = ui.spacing().item_spacing.x;
                let control_width = SETTINGS_CONTROL_WIDTH
                    .min((total_width * 0.40).max(SETTINGS_CONTROL_MIN_WIDTH));
                let text_width = (total_width - control_width - 30.0 - spacing * 2.0).max(160.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(48.0);
                    tool_symbol(ui, symbol, palette.surface_alt);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 48.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(description)
                                        .size(SETTINGS_FONT_SMALL)
                                        .color(palette.muted),
                                )
                                .wrap(),
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(control_width, 48.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(control_width);
                            ui.set_max_width(control_width);
                            add_control(ui);
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

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

#[derive(Clone, Copy)]
enum StatusTone {
    Info,
    Success,
    Warning,
    Danger,
}

fn status_tone_colors(palette: FluentPalette, tone: StatusTone) -> (Color32, Color32) {
    match tone {
        StatusTone::Info => (palette.accent, palette.info_bg),
        StatusTone::Success => (palette.success, palette.success_bg),
        StatusTone::Warning => (palette.warning, palette.warning_bg),
        StatusTone::Danger => (palette.danger, palette.danger_bg),
    }
}

fn status_badge(ui: &mut egui::Ui, tone: StatusTone, text: &str) {
    let palette = fluent_palette(ui);
    let (color, fill) = status_tone_colors(palette, tone);
    let max_width = ui.available_width().max(120.0);
    egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, color))
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.set_max_width(max_width);
            ui.horizontal_wrapped(|ui| {
                status_dot(ui, color);
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .size(SETTINGS_FONT_SMALL)
                            .color(palette.text),
                    )
                    .wrap(),
                );
            });
        });
}

fn inline_notice(ui: &mut egui::Ui, tone: StatusTone, text: &str) {
    let palette = fluent_palette(ui);
    let (color, fill) = status_tone_colors(palette, tone);
    egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, color))
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .size(SETTINGS_FONT_SMALL)
                            .color(palette.text),
                    )
                    .wrap(),
                );
            });
        });
}

fn diagnostic_status_card(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    let palette = fluent_palette(ui);
    egui::Frame::none()
        .fill(palette.surface_alt)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(42.0);
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(label)
                            .size(SETTINGS_FONT_SMALL)
                            .color(palette.muted),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(value)
                                .size(SETTINGS_FONT_SETTING_TITLE)
                                .color(palette.text),
                        )
                        .truncate(),
                    )
                    .on_hover_text(value);
                });
            });
        });
}

fn outline_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let palette = fluent_palette(ui);
    ui.add(
        egui::Button::new(label)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, palette.border))
            .rounding(4.0)
            .min_size(egui::vec2(SETTINGS_BUTTON_WIDTH, 30.0)),
    )
}

fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let palette = fluent_palette(ui);
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(palette.danger))
            .fill(palette.danger_bg)
            .stroke(Stroke::new(1.0, palette.danger))
            .rounding(4.0)
            .min_size(egui::vec2(SETTINGS_BUTTON_WIDTH, 30.0)),
    )
}

fn setting_toggle(ui: &mut egui::Ui, title: &str, description: &str, value: &mut bool) {
    setting_row(ui, title, description, |ui| {
        capsule_switch(ui, value);
    });
}

pub(super) fn capsule_switch(ui: &mut egui::Ui, value: &mut bool) -> egui::Response {
    let palette = fluent_palette(ui);
    let size = egui::vec2(50.0, 28.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }

    let fill = if *value {
        if response.is_pointer_button_down_on() {
            palette.accent_pressed
        } else if response.hovered() {
            palette.accent_hover
        } else {
            palette.accent
        }
    } else if response.hovered() {
        palette.control_hover
    } else {
        palette.control_bg
    };
    let stroke = if *value {
        Stroke::new(1.0, fill)
    } else {
        Stroke::new(1.0, palette.border)
    };
    let radius = rect.height() / 2.0;
    ui.painter().rect(rect, radius, fill, stroke);

    let knob_radius = 10.5;
    let knob_x = if *value {
        rect.right() - radius
    } else {
        rect.left() + radius
    };
    let knob_fill = if *value {
        palette.accent_text
    } else {
        palette.text
    };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, rect.center().y), knob_radius, knob_fill);

    response
}

fn setting_slider_usize(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut usize,
    range: std::ops::RangeInclusive<usize>,
) {
    setting_row(ui, title, description, |ui| {
        ui.add_sized([230.0, 20.0], Slider::new(value, range).show_value(true));
    });
}

fn setting_slider_f64(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) {
    setting_row(ui, title, description, |ui| {
        ui.add_sized([230.0, 20.0], Slider::new(value, range).show_value(true));
    });
}

fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let palette = fluent_palette(ui);
    let row = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(0.0, 5.0))
        .show(ui, |ui| {
            let total_width = ui.available_width();
            if total_width < SETTINGS_ROW_STACK_WIDTH {
                ui.vertical(|ui| {
                    ui.set_width(total_width);
                    ui.label(
                        RichText::new(title)
                            .strong()
                            .size(SETTINGS_FONT_SETTING_TITLE)
                            .color(palette.text),
                    );
                    if !description.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(description)
                                    .size(SETTINGS_FONT_SMALL)
                                    .color(palette.muted),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(6.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(total_width, 34.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(total_width);
                            add_control(ui);
                        },
                    );
                });
            } else {
                let spacing = ui.spacing().item_spacing.x;
                let control_width = SETTINGS_CONTROL_WIDTH
                    .min((total_width * 0.42).max(SETTINGS_CONTROL_MIN_WIDTH));
                let text_width = (total_width - control_width - spacing).max(160.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(46.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 46.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            if !description.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(description)
                                            .size(SETTINGS_FONT_SMALL)
                                            .color(palette.muted),
                                    )
                                    .wrap(),
                                );
                            }
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(control_width, 46.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(control_width);
                            ui.set_max_width(control_width);
                            add_control(ui);
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

fn setting_combo_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    selected_text: impl Into<egui::WidgetText>,
    id: impl std::hash::Hash,
    add_options: impl FnOnce(&mut egui::Ui),
) {
    setting_row(ui, title, description, |ui| {
        ComboBox::from_id_salt(id)
            .selected_text(selected_text)
            .width(230.0)
            .show_ui(ui, add_options);
    });
}

fn input_habits_ui(ui: &mut egui::Ui, model: &mut SettingsModel) {
    section_panel(ui, "模式默认值", |ui| {
        setting_toggle(
            ui,
            "默认英文模式",
            "打开输入法时默认进入英文直输。",
            &mut model.default_ascii,
        );
        setting_toggle(
            ui,
            "全局英文模式",
            "中英状态在所有应用之间共享。",
            &mut model.global_ascii,
        );
        setting_toggle(
            ui,
            "中文模式下键入全角字符",
            "中文模式启动后默认使用全角字母和符号；Shift+Space 可临时切换。",
            &mut model.default_full_shape,
        );
        setting_toggle(
            ui,
            "默认中文标点",
            "启动后默认把常用标点转换为中文标点。",
            &mut model.default_chinese_punct,
        );
        setting_toggle(
            ui,
            "中文标点使用弯引号",
            "中文引号优先输出成“”和‘’。",
            &mut model.curly_punct,
        );
        setting_toggle(
            ui,
            "开启符号自动补全",
            "输入括号或引号时自动补齐成一对，并把光标放到中间。",
            &mut model.auto_pair_punct,
        );
        setting_toggle(
            ui,
            "数字全角",
            "中文模式下把数字键转换为全角数字。",
            &mut model.number_fullwidth,
        );
        setting_toggle(
            ui,
            "符号全角",
            "拼音模式下把括号、斜杠和 ASCII 符号转换为全角符号。",
            &mut model.symbol_fullwidth,
        );
        setting_toggle(
            ui,
            "Shift 符号临时英文直出",
            "按住 Shift 输入数字和符号时保留键盘原始 ASCII 符号。",
            &mut model.shift_symbol_temporary_ascii,
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "拼音能力", |ui| {
        setting_toggle(
            ui,
            "默认模糊音",
            "启动后默认启用 z/zh、s/sh 等模糊匹配。",
            &mut model.default_fuzzy_pinyin,
        );
        ui.collapsing("模糊音细项", |ui| {
            setting_toggle(
                ui,
                "z / zh",
                "允许 z 和 zh 双向模糊匹配。",
                &mut model.fuzzy_zh_z,
            );
            setting_toggle(
                ui,
                "c / ch",
                "允许 c 和 ch 双向模糊匹配。",
                &mut model.fuzzy_ch_c,
            );
            setting_toggle(
                ui,
                "s / sh",
                "允许 s 和 sh 双向模糊匹配。",
                &mut model.fuzzy_sh_s,
            );
            setting_toggle(
                ui,
                "n / l",
                "允许 n 和 l 双向模糊匹配。",
                &mut model.fuzzy_n_l,
            );
            setting_toggle(
                ui,
                "f / h",
                "允许 f 和 h 双向模糊匹配。",
                &mut model.fuzzy_f_h,
            );
            setting_toggle(
                ui,
                "an / ang",
                "允许 an 和 ang 双向模糊匹配。",
                &mut model.fuzzy_an_ang,
            );
            setting_toggle(
                ui,
                "en / eng",
                "允许 en 和 eng 双向模糊匹配。",
                &mut model.fuzzy_en_eng,
            );
            setting_toggle(
                ui,
                "in / ing",
                "允许 in 和 ing 双向模糊匹配。",
                &mut model.fuzzy_in_ing,
            );
        });
        setting_toggle(
            ui,
            "默认双拼",
            "启动后默认使用双拼解析输入。",
            &mut model.default_double_pinyin,
        );
        setting_toggle(
            ui,
            "简拼",
            "允许用声母组合快速输入长词。",
            &mut model.jianpin,
        );
        setting_toggle(
            ui,
            "混拼",
            "允许全拼和声母混合输入词语，默认使用保守匹配。",
            &mut model.mixed_pinyin,
        );
        setting_toggle(
            ui,
            "宽松混拼",
            "放开低可信混拼路径和更多短输入展开，可能增加重码。",
            &mut model.mixed_pinyin_aggressive,
        );
        setting_toggle(
            ui,
            "启用纠错",
            "键盘邻键、漏键等常见拼写错误会参与候选排序。",
            &mut model.correction_enabled,
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "输入辅助", |ui| {
        setting_toggle(
            ui,
            "日期自动格式化",
            "rq、sj 等快捷输入始终使用实时日期时间。",
            &mut model.date_auto_format,
        );
        setting_toggle(
            ui,
            "失败时重试",
            "引擎忙或宿主拒绝编辑时尝试延迟刷新。",
            &mut model.retry_on_failure,
        );
    });
}

fn input_tools_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let mut open_handwrite = false;
    quiet_section(ui, "输入辅助工具", |ui| {
        tool_row(
            ui,
            "VV",
            "VV 直输助手",
            "启用日期、时间、符号、网址和 Markdown 等直输命令。",
            |ui| {
                capsule_switch(ui, &mut app.model.v_assist);
            },
        );
        tool_row(
            ui,
            "SYM",
            "符号输入工具箱",
            "使用 vv sym/fh 打开常用标点、括号、箭头、数学和编号符号；支持 dunhao、douhao 等单个符号名。",
            |ui| {
                capsule_switch(ui, &mut app.model.symbol_toolbox);
            },
        );
        tool_row(
            ui,
            "EM",
            "Emoji 表情输入",
            "使用 vv emoji、vv emoji smile 或 vv sym emoji 输入单个 emoji 表情候选。",
            |ui| {
                capsule_switch(ui, &mut app.model.emoji_input);
            },
        );
        tool_row(
            ui,
            "U",
            "U 模式拆字",
            "使用部件编码辅助输入不认识读音的汉字。",
            |ui| {
                capsule_switch(ui, &mut app.model.u_mode);
            },
        );
        tool_row(
            ui,
            "EN",
            "英语单词输入",
            "中文模式下输入英文前缀时，启用内置 2 万词常用英文候选。",
            |ui| {
                capsule_switch(ui, &mut app.model.english_word_input);
            },
        );
        tool_row(
            ui,
            "中",
            "状态提示",
            "显示中英、全角、标点等输入状态提示。",
            |ui| {
                capsule_switch(ui, &mut app.model.show_status_notifications);
            },
        );
        tool_row(
            ui,
            "手",
            "手写查字",
            "独立浮窗，识别后可复制或粘贴候选字。",
            |ui| {
                if outline_button(ui, "打开")
                    .on_hover_text("打开手写查字")
                    .clicked()
                {
                    open_handwrite = true;
                }
            },
        );
    });

    if open_handwrite {
        app.open_handwrite();
    }
    ui.add_space(10.0);
    section_panel(ui, "系统输入法", |ui| {
        let palette = fluent_palette(ui);
        ui.label(
            RichText::new("管理 Windows 用户语言列表中的常用输入法和键盘。")
                .small()
                .color(palette.muted),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if outline_button(ui, "添加微软拼音").clicked() {
                app.add_microsoft_pinyin();
            }
            if outline_button(ui, "删除微软拼音").clicked() {
                app.remove_microsoft_pinyin();
            }
            if outline_button(ui, "添加美式键盘").clicked() {
                app.add_us_keyboard();
            }
            if outline_button(ui, "删除美式键盘").clicked() {
                app.remove_us_keyboard();
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if outline_button(ui, "置顶开心输入法").clicked() {
                app.pin_kaixin_input();
            }
            if outline_button(ui, "取消置顶").clicked() {
                app.unpin_kaixin_input();
            }
        });
    });

    ui.add_space(10.0);
    section_panel(ui, "VV 命令速查", |ui| {
        let palette = fluent_palette(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new("筛选").small().color(palette.muted));
            ui.add_sized(
                [260.0, 26.0],
                TextEdit::singleline(&mut app.vv_command_filter).hint_text("mail / rq / md"),
            );
        });
        ui.add_space(6.0);
        let filter = app.vv_command_filter.trim().to_lowercase();
        egui::Grid::new("vv_command_help")
            .num_columns(4)
            .striped(true)
            .spacing([18.0, 7.0])
            .show(ui, |ui| {
                ui.strong("命令");
                ui.strong("用途");
                ui.strong("示例");
                ui.strong("学习");
                ui.end_row();
                for (command, usage, example) in VV_COMMAND_HELP {
                    if !filter.is_empty() {
                        let haystack = format!("{command} {usage} {example}").to_lowercase();
                        if !haystack.contains(&filter) {
                            continue;
                        }
                    }
                    ui.monospace(*command);
                    ui.label(*usage);
                    ui.monospace(*example);
                    ui.label(RichText::new("不记录").small().color(palette.muted));
                    ui.end_row();
                }
            });
    });
}

fn custom_shortcuts_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "自定义短语 / 快捷码", |ui| {
        let palette = fluent_palette(ui);
        ui.label(
            RichText::new("每行一个：快捷码 = 候选文本。多个候选可用 | 分隔；换行可写成 \\n。")
                .small()
                .color(palette.muted),
        );
        ui.add_space(6.0);
        egui::Frame::none()
            .fill(palette.surface_alt)
            .stroke(Stroke::new(1.0, palette.border_subtle))
            .rounding(4.0)
            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
            .show(ui, |ui| {
                ui.add(
                    TextEdit::multiline(&mut app.model.custom_shortcuts)
                        .font(TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(7)
                        .hint_text(
                            ";qq = name@example.com\\n;;r = 此致，敬礼\\n;sig = 此致\\n敬礼",
                        ),
                );
            });
        ui.label(
            RichText::new(
                "保存后直接输入 ;qq 或 ;;r 即可出候选；也可以输入 vv ;qq 调出同一组候选。",
            )
            .small()
            .color(palette.muted),
        );
    });
}

#[allow(dead_code)]
fn ocr_translation_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let mut open_ocr = false;
    let mut open_ocr_translate = false;
    let mut open_translate = false;
    let mut check_ocr = false;
    let mut check_translate = false;

    quiet_section(ui, "OCR 与翻译", |ui| {
        tool_row(
            ui,
            "OCR",
            "截图 OCR",
            "截取屏幕区域后用本地 RapidOCR 识别文字，也可以直接送入翻译器。",
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if outline_button(ui, "打开")
                        .on_hover_text("打开截图 OCR")
                        .clicked()
                    {
                        open_ocr = true;
                    }
                    if outline_button(ui, "翻译")
                        .on_hover_text("截图 OCR 后打开中英翻译")
                        .clicked()
                    {
                        open_ocr_translate = true;
                    }
                    if outline_button(ui, "检测")
                        .on_hover_text("检测 RapidOCR 环境")
                        .clicked()
                    {
                        check_ocr = true;
                    }
                });
            },
        );
        if rapidocr_paths::rapidocr_root().is_none() {
            inline_notice(
                ui,
                StatusTone::Warning,
                "RapidOCR 环境未找到，OCR 暂不可用。点“检测”可查看缺失项。",
            );
            ui.add_space(6.0);
        }
        tool_row(
            ui,
            "译",
            "中英翻译",
            "选中文本后发送到独立 WinTranslator，并自动开始翻译。",
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if outline_button(ui, "打开")
                        .on_hover_text("打开独立翻译软件")
                        .clicked()
                    {
                        open_translate = true;
                    }
                    if outline_button(ui, "检测")
                        .on_hover_text("检测 WinTranslator 安装或运行状态")
                        .clicked()
                    {
                        check_translate = true;
                    }
                });
            },
        );
        if !translation_available() {
            inline_notice(
                ui,
                StatusTone::Warning,
                "未找到 WinTranslator。点“检测”可查看安装提示。",
            );
            ui.add_space(6.0);
        }
        executable_path_row(
            ui,
            "WinTranslator 路径",
            "可留空自动检测；自定义安装目录时选择 WinTranslator.exe。保存后“检测”会测试联动状态（协议 v2）。",
            &mut app.model.wintranslator_path,
            "自动检测",
        );
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
            "开启后首次识别会加载模型，之后复用以减少等待；关闭后每次识别结束释放模型以降低内存占用。",
            &mut app.model.ocr_keep_alive,
        );
        setting_combo_row(
            ui,
            "OCR 速度与精度",
            "快速适合屏幕短文本；高精度使用更大的检测边长。",
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
            "开启后会保留截图预览和识别文字，方便校对；关闭后打开翻译器后自动收起 OCR 窗口。",
            &mut app.model.ocr_translate_keep_window,
        );
        setting_toggle(
            ui,
            "OCR 截图自动保存",
            "开启后，截图 OCR 会另存一份原图到本地目录；不影响 OCR 预览临时图。",
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

    if open_ocr {
        app.open_ocr();
    }
    if open_ocr_translate {
        app.open_ocr_translate();
    }
    if open_translate {
        app.open_translate();
    }
    if check_ocr {
        app.check_ocr_language();
    }
    if check_translate {
        app.check_translation_environment();
    }
}

#[derive(Clone, Copy)]
struct CandidatePreviewColors {
    window: Color32,
    header: Color32,
    border: Color32,
    divider: Color32,
    item: Color32,
    item_border: Color32,
    selected: Color32,
    selected_border: Color32,
    text: Color32,
    muted: Color32,
    selected_text: Color32,
    selected_muted: Color32,
    chip: Color32,
    chip_border: Color32,
    chip_text: Color32,
}

fn parse_skin_color(value: &str, fallback: Color32) -> Color32 {
    let value = value.trim().trim_start_matches('#');
    match value.len() {
        6 => u32::from_str_radix(value, 16).ok().map(|rgb| {
            Color32::from_rgb(
                ((rgb >> 16) & 0xff) as u8,
                ((rgb >> 8) & 0xff) as u8,
                (rgb & 0xff) as u8,
            )
        }),
        8 => u32::from_str_radix(value, 16).ok().map(|argb| {
            Color32::from_rgba_unmultiplied(
                ((argb >> 16) & 0xff) as u8,
                ((argb >> 8) & 0xff) as u8,
                (argb & 0xff) as u8,
                ((argb >> 24) & 0xff) as u8,
            )
        }),
        _ => None,
    }
    .unwrap_or(fallback)
}

fn skin_key_from_config(value: &str) -> Option<String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }
    let normalized = normalized.trim_end_matches('/');
    let normalized = if normalized
        .rsplit('/')
        .next()
        .is_some_and(|part| part.eq_ignore_ascii_case("theme.json"))
    {
        normalized
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("")
    } else {
        normalized
    };
    normalized
        .rsplit('/')
        .next()
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

fn selected_skin<'a>(model: &SettingsModel, skins: &'a [SkinPreview]) -> Option<&'a SkinPreview> {
    let configured = model.candidate_skin_file.trim();
    let configured_key = skin_key_from_config(configured);
    skins.iter().find(|skin| {
        skin.key.eq_ignore_ascii_case(configured)
            || configured_key
                .as_deref()
                .is_some_and(|key| skin.key.eq_ignore_ascii_case(key))
    })
}

fn preview_colors_for_skin(
    palette: FluentPalette,
    skin: Option<&SkinPreview>,
) -> CandidatePreviewColors {
    if let Some(skin) = skin {
        let window = parse_skin_color(&skin.window_bg, palette.surface);
        return CandidatePreviewColors {
            window,
            header: parse_skin_color(&skin.header_bg, window),
            border: parse_skin_color(&skin.border, palette.border),
            divider: parse_skin_color(&skin.divider, palette.border),
            item: parse_skin_color(&skin.item_bg, window),
            item_border: parse_skin_color(&skin.item_border, palette.border),
            selected: parse_skin_color(&skin.selected_bg, palette.nav_selected),
            selected_border: parse_skin_color(&skin.selected_border, palette.accent),
            text: parse_skin_color(&skin.text, palette.text),
            muted: parse_skin_color(&skin.muted_text, palette.muted),
            selected_text: parse_skin_color(&skin.selected_text, palette.text),
            selected_muted: parse_skin_color(&skin.selected_muted_text, palette.muted),
            chip: parse_skin_color(&skin.chip_bg, palette.surface_alt),
            chip_border: parse_skin_color(&skin.chip_border, palette.border),
            chip_text: parse_skin_color(&skin.chip_text, palette.text),
        };
    }
    CandidatePreviewColors {
        window: palette.surface,
        header: palette.surface_alt,
        border: palette.border,
        divider: palette.border_subtle,
        item: palette.surface,
        item_border: palette.border,
        selected: palette.nav_selected,
        selected_border: palette.accent,
        text: palette.text,
        muted: palette.muted,
        selected_text: palette.text,
        selected_muted: palette.muted,
        chip: palette.surface_alt,
        chip_border: palette.border,
        chip_text: palette.text,
    }
}

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
    let count = metrics.count;

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
            painter.rect(
                card,
                6.0,
                if selected {
                    colors.selected
                } else {
                    colors.item
                },
                Stroke::new(
                    1.0,
                    if selected {
                        colors.selected_border
                    } else {
                        colors.item_border
                    },
                ),
            );
            painter.text(
                egui::pos2(card.left() + 7.0, card.top() + 10.0),
                egui::Align2::LEFT_TOP,
                format!("{} {}", idx + 1, candidate),
                FontId::proportional(font_size),
                if selected {
                    colors.selected_text
                } else {
                    colors.text
                },
            );
            if metrics.has_comment {
                painter.text(
                    egui::pos2(card.left() + 7.0, card.bottom() - 9.0),
                    egui::Align2::LEFT_BOTTOM,
                    *meta,
                    FontId::proportional(SETTINGS_MIN_HINT_FONT.max(11.0)),
                    if selected {
                        colors.selected_muted
                    } else {
                        colors.muted
                    },
                );
            }
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
            painter.rect(
                row,
                5.0,
                if selected {
                    colors.selected
                } else {
                    colors.item
                },
                Stroke::new(
                    1.0,
                    if selected {
                        colors.selected_border
                    } else {
                        colors.item_border
                    },
                ),
            );
            painter.text(
                egui::pos2(
                    row.left() + 9.0,
                    if metrics.has_comment {
                        row.top() + 7.0
                    } else {
                        row.center().y
                    },
                ),
                if metrics.has_comment {
                    egui::Align2::LEFT_TOP
                } else {
                    egui::Align2::LEFT_CENTER
                },
                format!("{}   {}", idx + 1, candidate),
                FontId::proportional(font_size),
                if selected {
                    colors.selected_text
                } else {
                    colors.text
                },
            );
            if metrics.has_comment {
                painter.text(
                    egui::pos2(row.left() + 9.0, row.bottom() - 6.0),
                    egui::Align2::LEFT_BOTTOM,
                    *meta,
                    FontId::proportional(SETTINGS_MIN_HINT_FONT.max(11.0)),
                    if selected {
                        colors.selected_muted
                    } else {
                        colors.muted
                    },
                );
            }
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
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 92.0), egui::Sense::click());
    let outer_stroke = if selected || response.hovered() {
        Stroke::new(1.5, palette.accent)
    } else {
        Stroke::new(1.0, palette.border_subtle)
    };
    ui.painter()
        .rect(rect, 7.0, palette.surface_alt, outer_stroke);
    let preview = egui::Rect::from_min_max(
        rect.min + egui::vec2(5.0, 5.0),
        egui::pos2(rect.right() - 5.0, rect.bottom() - 27.0),
    );
    ui.painter()
        .rect(preview, 5.0, colors.window, Stroke::new(1.0, colors.border));
    let normal = egui::Rect::from_min_size(
        preview.min + egui::vec2(5.0, 6.0),
        egui::vec2(preview.width() - 10.0, 18.0),
    );
    let active = normal.translate(egui::vec2(0.0, 22.0));
    ui.painter().rect(
        normal,
        4.0,
        colors.item,
        Stroke::new(1.0, colors.item_border),
    );
    ui.painter().rect(
        active,
        4.0,
        colors.selected,
        Stroke::new(1.0, colors.selected_border),
    );
    ui.painter().text(
        normal.left_center() + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "1 输入法",
        FontId::proportional(SETTINGS_MIN_HINT_FONT.max(10.5)),
        colors.text,
    );
    ui.painter().text(
        active.left_center() + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "2 输入",
        FontId::proportional(SETTINGS_MIN_HINT_FONT.max(10.5)),
        colors.selected_text,
    );
    let title = if key.is_empty() { "默认" } else { label };
    ui.painter().text(
        egui::pos2(rect.left() + 7.0, rect.bottom() - 10.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(SETTINGS_FONT_SMALL.max(SETTINGS_MIN_HINT_FONT)),
        if selected {
            palette.accent
        } else {
            palette.text
        },
    );
    response.on_hover_text(if key.is_empty() {
        "使用内置默认外观"
    } else {
        key
    })
}

const CORE_CANDIDATE_SKINS: &[&str] = &["dark", "mint-glass", "high-visibility", "retro-terminal"];

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
                    "默认",
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
    let core: Vec<&SkinPreview> = CORE_CANDIDATE_SKINS
        .iter()
        .filter_map(|key| skins.iter().find(|skin| skin.key == *key))
        .collect();
    let more: Vec<&SkinPreview> = skins
        .iter()
        .filter(|skin| !CORE_CANDIDATE_SKINS.contains(&skin.key.as_str()))
        .collect();

    skin_card_grid_rows(ui, model, &core, "candidate_skin_core_cards", true);
    if !more.is_empty() {
        ui.add_space(6.0);
        ui.collapsing(format!("更多皮肤（{}）", more.len()), |ui| {
            ui.add_space(4.0);
            skin_card_grid_rows(ui, model, &more, "candidate_skin_more_cards", false);
        });
    }
}

fn candidate_appearance_ui(
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
            RichText::new("点击色板即可切换；上方候选栏会立即预览实际配色。")
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

fn hotkeys_ui(ui: &mut egui::Ui, model: &mut SettingsModel) {
    let translate_available = translation_available();
    section_panel(ui, "快捷键", |ui| {
        setting_combo_row(
            ui,
            "中英切换",
            "切换中文输入与英文直输模式。",
            cn_en_hotkey_label(&model.cn_en_hotkey),
            "cn_en_hotkey",
            |ui| {
                selectable_string(
                    ui,
                    &mut model.cn_en_hotkey,
                    "both",
                    "Ctrl+Shift / Ctrl+Space",
                );
                selectable_string(ui, &mut model.cn_en_hotkey, "shift", "Ctrl+Shift");
                selectable_string(ui, &mut model.cn_en_hotkey, "ctrl_space", "Ctrl+Space");
                selectable_string(ui, &mut model.cn_en_hotkey, "none", "关闭");
            },
        );
        setting_toggle(
            ui,
            "全半角切换",
            "使用 Shift+Space 在全角和半角之间切换；默认全角开启时会自动启用。",
            &mut model.full_shape_hotkey,
        );
        setting_toggle(
            ui,
            "标点切换",
            "使用 Ctrl+. 在中文标点和英文标点之间切换。",
            &mut model.punct_hotkey,
        );
        setting_toggle(
            ui,
            "模糊音切换",
            "使用 Ctrl+Shift+F 快速开关模糊音。",
            &mut model.fuzzy_hotkey,
        );
        setting_toggle(
            ui,
            "双拼切换",
            "使用 Ctrl+Shift+D 快速开关双拼。",
            &mut model.double_pinyin_hotkey,
        );
        setting_toggle(
            ui,
            "轻按 Shift 切换英文",
            "单击 Shift 临时进入英文直输。",
            &mut model.shift_tap_hotkey,
        );
        setting_toggle(
            ui,
            "默认繁体输出",
            "开启后候选输出为繁体；也可使用下方快捷键临时切换。",
            &mut model.traditional_output,
        );
        setting_row(
            ui,
            "简繁切换",
            "在当前输入法会话中切换简体/繁体输出。",
            |ui| {
                hotkey_combo(
                    ui,
                    "",
                    "traditional_hotkey",
                    &mut model.traditional_hotkey,
                    "F",
                );
            },
        );
        setting_row(
            ui,
            "游戏兼容模式",
            "手动切换游戏兼容策略，适合临时进入全屏游戏前使用。",
            |ui| {
                hotkey_combo(ui, "", "game_mode_hotkey", &mut model.game_mode_hotkey, "G");
            },
        );
        setting_row(
            ui,
            "临时强制英文",
            "手动切换临时英文直输。",
            |ui| {
                hotkey_combo(
                    ui,
                    "",
                    "temporary_ascii_hotkey",
                    &mut model.temporary_ascii_hotkey,
                    "Space",
                );
            },
        );
        setting_toggle(
            ui,
            "候选数字直选",
            "按数字键直接提交对应候选。",
            &mut model.candidate_number_select,
        );
        setting_toggle(
            ui,
            "翻页 - / =",
            "使用 - 和 = 翻页。",
            &mut model.page_minus_equal,
        );
        setting_toggle(
            ui,
            "翻页 , / .",
            "使用逗号和句号翻页。",
            &mut model.page_comma_period,
        );
        setting_toggle(
            ui,
            "翻页 PgUp / PgDn",
            "使用 PageUp 和 PageDown 翻页。",
            &mut model.page_pgup_pgdn,
        );
        setting_row(
            ui,
            "截图快捷键",
            "组合键固定为修饰键 + 字母。",
            |ui| {
                ui.vertical(|ui| {
                    fixed_letter_hotkey_combo(
                        ui,
                        "",
                        "screenshot_hotkey_letter",
                        &mut model.screenshot_hotkey,
                        'A',
                    );
                    hotkey_registration_status_label(ui, "registered");
                });
            },
        );
        setting_row(ui, "设置页快捷键", "全局打开设置页。", |ui| {
            hotkey_combo(
                ui,
                "",
                "settings_hotkey",
                &mut model.settings_hotkey,
                "Comma",
            );
        });
        setting_row(
            ui,
            "手写查字快捷键",
            "全局打开手写查字工具。",
            |ui| {
                hotkey_combo(ui, "", "handwrite_hotkey", &mut model.handwrite_hotkey, "H");
            },
        );
        setting_row(ui, "OCR 快捷键", "全局打开 OCR 工具。", |ui| {
            ui.vertical(|ui| {
                hotkey_combo(ui, "", "ocr_hotkey", &mut model.ocr_hotkey, "O");
                hotkey_registration_status_label(ui, "ocr_registered");
            });
        });
        setting_row(
            ui,
            "截图翻译快捷键",
            "截图识别后自动送入中英翻译。",
            |ui| {
                ui.vertical(|ui| {
                    hotkey_combo(
                        ui,
                        "",
                        "ocr_translate_hotkey",
                        &mut model.ocr_translate_hotkey,
                        "Y",
                    );
                    hotkey_registration_status_label(ui, "ocr_translate_registered");
                });
            },
        );
        setting_row(
            ui,
            "中英翻译快捷键",
            "全局打开中英翻译浮窗。",
            |ui| {
                ui.vertical(|ui| {
                    hotkey_combo(ui, "", "translate_hotkey", &mut model.translate_hotkey, "T");
                    hotkey_registration_status_label(ui, "translate_registered");
                    if !translate_available {
                        status_badge(ui, StatusTone::Warning, "模型未安装");
                    }
                });
            },
        );
        let palette = fluent_palette(ui);
        ui.label(
            RichText::new("快捷键冲突会以橙色提示。")
                .small()
                .color(palette.muted),
        );
    });
}

fn clipboard_settings_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "剪贴板快捷键", |ui| {
        setting_row(
            ui,
            "剪贴板快捷键",
            "打开独立剪贴板管理器；组合键固定为修饰键 + 字母。",
            |ui| {
                fixed_letter_hotkey_combo(
                    ui,
                    "",
                    "clipboard_hotkey_letter",
                    &mut app.model.clipboard_hotkey,
                    'V',
                );
            },
        );
    });

    section_panel(ui, "历史记录", |ui| {
        setting_toggle(
            ui,
            "后台保存剪贴板历史",
            "关闭后不再记录系统剪贴板文本；已保存内容仍可在下方清空。",
            &mut app.model.clipboard_background_enabled,
        );
        setting_slider_usize(
            ui,
            "历史记录上限",
            "最多保留的普通剪贴板条数；设为 0 表示不保留普通历史。",
            &mut app.model.clipboard_max_history_items,
            0..=300,
        );
        setting_slider_usize(
            ui,
            "置顶记录上限",
            "最多保留的置顶剪贴板条数；设为 0 表示不保留置顶记录。",
            &mut app.model.clipboard_max_pinned_items,
            0..=100,
        );
        setting_slider_usize(
            ui,
            "单条文本上限",
            "单条文本最多保存的 UTF-16 单元数，过长内容会被截断。",
            &mut app.model.clipboard_max_text_utf16_units,
            20..=20_000,
        );
        setting_toggle(
            ui,
            "候选栏显示剪贴板片段",
            "关闭后仍可使用剪贴板候选，但候选栏不显示复制文本预览。",
            &mut app.model.clipboard_candidate_preview_enabled,
        );
        setting_toggle(
            ui,
            "记录来源应用",
            "开启后会在剪贴板历史中保存复制来源进程路径。",
            &mut app.model.clipboard_record_source_app,
        );
        setting_slider_usize(
            ui,
            "自动清理天数",
            "用于剪贴板管理器的自动清理参考；0 表示不按天数清理。",
            &mut app.model.clipboard_max_age_days,
            0..=3650,
        );
        setting_toggle(
            ui,
            "置顶也按天数清理",
            "开启后，按天数清理时也会处理置顶记录；清理天数为 0 时不按天数清理。",
            &mut app.model.clipboard_pinned_respects_max_age,
        );
    });

    let clipboard_path = pinyin_ime::clipboard_store::store_path();
    let clipboard_storage_status = clipboard_storage_status_text(&clipboard_path);
    section_panel(ui, "存储与维护", |ui| {
        setting_row(ui, "加密状态", &clipboard_storage_status, |_| {});
        data_location_row(
            ui,
            "历史数据库",
            "剪贴板历史仅在后台保存开启时写入本机数据库。",
            &clipboard_path,
            |ui| {
                if outline_button(ui, "打开").clicked() {
                    app.open_data_location(clipboard_path.clone());
                }
            },
        );
        inline_notice(
            ui,
            StatusTone::Danger,
            "清空会删除本机已保存的剪贴板历史和置顶项。",
        );
        ui.add_space(6.0);
        if danger_button(ui, "清空历史").clicked() {
            app.clear_clipboard();
        }
    });

    section_panel(ui, "敏感应用", |ui| {
        ui.label("以下进程名单会以明文保存到 config.ini；匹配后不解析剪贴板历史候选。");
        privacy_process_list_row(
            ui,
            "永不剪贴板",
            "匹配这些进程时，不解析剪贴板历史候选。",
            &mut app.model.privacy_never_clipboard_processes,
        );
    });
}

struct HotkeyRegistrationStatus {
    enabled: bool,
    hotkey: Option<String>,
    reason: Option<String>,
    error: Option<String>,
}

fn hotkey_registration_status_label(ui: &mut egui::Ui, status_key: &str) {
    let (text, tone) = match read_hotkey_registration_status(status_key) {
        Some(status) if status.enabled => {
            let hotkey = status.hotkey.unwrap_or_else(|| "未知快捷键".to_string());
            (format!("已生效 {hotkey}"), StatusTone::Success)
        }
        Some(status) if status.reason.as_deref() == Some("disabled") => {
            ("已关闭".to_string(), StatusTone::Info)
        }
        Some(status) => {
            let detail = status
                .error
                .or(status.reason)
                .unwrap_or_else(|| "注册失败".to_string());
            (format!("注册失败 {detail}"), StatusTone::Warning)
        }
        None => ("等待托盘注册".to_string(), StatusTone::Info),
    };
    status_badge(ui, tone, &text);
}

fn read_hotkey_registration_status(status_key: &str) -> Option<HotkeyRegistrationStatus> {
    let path = app_paths::local_data_dir()?.join("screenshot_hotkey_status.txt");
    let body = fs::read_to_string(path).ok()?;
    parse_hotkey_registration_status(&body, status_key)
}

fn parse_hotkey_registration_status(
    body: &str,
    status_key: &str,
) -> Option<HotkeyRegistrationStatus> {
    let tokens = body.split_whitespace().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < tokens.len() {
        let Some((key, value)) = tokens[idx].split_once('=') else {
            idx += 1;
            continue;
        };
        if key != status_key {
            idx += 1;
            continue;
        }

        let mut status = HotkeyRegistrationStatus {
            enabled: value == "1",
            hotkey: None,
            reason: None,
            error: None,
        };
        idx += 1;
        while idx < tokens.len() {
            let Some((token_key, token_value)) = tokens[idx].split_once('=') else {
                idx += 1;
                continue;
            };
            if is_hotkey_status_start_token(token_key) {
                break;
            }
            match token_key {
                "hotkey" => status.hotkey = Some(token_value.to_string()),
                "reason" => status.reason = Some(token_value.to_string()),
                "error" => status.error = Some(token_value.to_string()),
                _ => {}
            }
            idx += 1;
        }
        return Some(status);
    }
    None
}

fn is_hotkey_status_start_token(key: &str) -> bool {
    key == "registered" || key.ends_with("_registered")
}

fn lexicon_learning_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "词库与学习", |ui| {
        lexicon_learning_controls(ui, app);
    });
}

fn lexicon_learning_controls(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.label(
        RichText::new("词库与学习")
            .strong()
            .size(SETTINGS_FONT_SECTION_TITLE),
    );
    setting_combo_row(
        ui,
        "学习灵敏度",
        "控制逐字拼出生词后写入用户词库的速度。保守更少误学，积极会更快置顶刚学词。",
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
    ui.horizontal_wrapped(|ui| {
        if outline_button(ui, "导入").clicked() {
            app.import_user_dict();
        }
        if outline_button(ui, "导出明文").clicked() {
            app.export_user_dict();
        }
        if outline_button(ui, "解密导出").clicked() {
            app.export_decrypted_user_dict();
        }
    });
    ui.label(
        RichText::new("导出的用户词库是明文文件，请妥善保管，使用后及时删除。")
            .color(palette.muted),
    );

    ui.add_space(6.0);
    ui.label(
        RichText::new("自定义短语")
            .strong()
            .size(SETTINGS_FONT_SECTION_TITLE),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label("编码");
        ui.add(TextEdit::singleline(&mut app.user_phrase_key).desired_width(120.0));
        ui.label("短语");
        ui.add(TextEdit::singleline(&mut app.user_phrase_text).desired_width(180.0));
        if outline_button(ui, "添加").clicked() {
            app.add_user_phrase();
        }
    });
    ui.label(
        RichText::new("导出的用户词库为明文 SQLite，请只保存到可信位置。")
            .small()
            .color(palette.muted),
    );

    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("永远不学")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        ui.add(TextEdit::singleline(&mut app.blocked_phrase_text).desired_width(180.0));
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

    if !app.model.lexicon_tags.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new("仅 zh-ext 扩展词库可以关闭；zh 热门主词库始终启用。关闭不常用扩展可让日常候选更靠前。")
                .small()
                .color(palette.muted),
        );
        ui.label(
            RichText::new("扩展词库（可关闭）")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        ui.horizontal_wrapped(|ui| {
            for (tag, enabled) in &mut app.model.lexicon_tags {
                ui.checkbox(enabled, lexicon_tag_label(tag));
            }
        });
        ui.horizontal_wrapped(|ui| {
            if outline_button(ui, "导入词库").clicked() {
                app.import_lexicon_text();
            }
            if outline_button(ui, "重新加载").clicked() {
                app.reload_lexicon_now();
            }
            ui.label(
                RichText::new("手动替换词库文件或重新 bake 后，可在这里立即刷新引擎。")
                    .small()
                    .color(palette.muted),
            );
        });
    }
}

#[derive(Clone, Copy)]
enum GameTestWizardAction {
    ApplyRecommended,
    ActivateGame,
    CandidateVisible,
    CandidateMissing,
    PositionGood,
    AdjustPosition,
    CommitGood,
    UseUnicode,
    UseClipboard,
    Close,
}

fn game_test_wizard_ui(ctx: &egui::Context, app: &mut SettingsApp) {
    let Some(wizard) = app.game_test_wizard.clone() else {
        return;
    };
    let mut open = true;
    let mut action = None;
    let dialog_bounds = ctx.screen_rect().shrink(12.0);
    egui::Window::new("游戏候选栏测试向导")
        .collapsible(false)
        .resizable(false)
        .default_width(520.0_f32.min(dialog_bounds.width()))
        .max_width(dialog_bounds.width())
        .max_height(dialog_bounds.height())
        .constrain_to(dialog_bounds)
        .scroll([false, true])
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            let palette = fluent_palette(ui);
            ui.label(
                RichText::new(&wizard.process)
                    .strong()
                    .size(SETTINGS_FONT_SECTION_TITLE),
            );
            if !wizard.title.is_empty() {
                ui.label(RichText::new(&wizard.title).small().color(palette.muted));
            }
            ui.add_space(8.0);

            match wizard.step {
                GameTestStep::Prepare => {
                    ui.label("第一步：应用游戏推荐配置并保存。该配置会显示候选栏、启用紧凑游戏档，先测试标准 TSF 上屏，并自动选择 Overlay 后端。");
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("应用推荐配置并保存").clicked() {
                            action = Some(GameTestWizardAction::ApplyRecommended);
                        }
                        if ui.button("取消").clicked() {
                            action = Some(GameTestWizardAction::Close);
                        }
                    });
                }
                GameTestStep::CandidateVisibility => {
                    let backend = matching_compat_rule(&app.model.compat_rules, &wizard.process)
                        .map(|rule| overlay_backend_label(&rule.overlay_backend))
                        .unwrap_or("自动（全屏时独立）");
                    ui.label("第二步：切换到游戏，切到中文模式并键入“nihao”，观察候选栏是否出现。");
                    ui.label(
                        RichText::new(format!("当前 Overlay 后端：{backend}"))
                            .small()
                            .color(palette.muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("切换到游戏窗口").clicked() {
                            action = Some(GameTestWizardAction::ActivateGame);
                        }
                        if ui.button("能看到候选").clicked() {
                            action = Some(GameTestWizardAction::CandidateVisible);
                        }
                        if ui.button("看不到候选，尝试独立 Overlay").clicked() {
                            action = Some(GameTestWizardAction::CandidateMissing);
                        }
                    });
                    ui.label(
                        RichText::new("真独占全屏可能阻止普通候选窗显示；优先使用无边框全屏。")
                            .small()
                            .color(palette.muted),
                    );
                }
                GameTestStep::Position => {
                    let summary = matching_compat_rule(&app.model.compat_rules, &wizard.process)
                        .map(|rule| {
                            format!(
                                "{}，偏移 ({}, {})，缩放 {}%，{}",
                                overlay_anchor_label(&rule.overlay_anchor),
                                rule.overlay_offset_x,
                                rule.overlay_offset_y,
                                rule.overlay_scale_percent,
                                overlay_monitor_label(&rule.overlay_monitor)
                            )
                        })
                        .unwrap_or_else(|| "自动位置".to_string());
                    ui.label("第三步：确认候选栏的位置、大小和目标显示器是否合适。");
                    ui.label(RichText::new(summary).small().color(palette.muted));
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("位置合适").clicked() {
                            action = Some(GameTestWizardAction::PositionGood);
                        }
                        if ui.button("返回规则页调整位置").clicked() {
                            action = Some(GameTestWizardAction::AdjustPosition);
                        }
                    });
                }
                GameTestStep::Commit => {
                    ui.label("第四步：在游戏聊天框输入并上屏一段中文，确认游戏能正常接收文本。");
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("上屏正常，完成").clicked() {
                            action = Some(GameTestWizardAction::CommitGood);
                        }
                        if ui.button("尝试 Unicode SendInput").clicked() {
                            action = Some(GameTestWizardAction::UseUnicode);
                        }
                        if ui.button("尝试剪贴板粘贴").clicked() {
                            action = Some(GameTestWizardAction::UseClipboard);
                        }
                    });
                }
            }
        });

    if !open {
        app.game_test_wizard = None;
        return;
    }
    match action {
        Some(GameTestWizardAction::ApplyRecommended) => {
            app.set_game_profile_for_process(&wizard.process);
            if app.save().is_ok() {
                if let Some(state) = app.game_test_wizard.as_mut() {
                    state.step = GameTestStep::CandidateVisibility;
                }
            }
        }
        Some(GameTestWizardAction::ActivateGame) => {
            match activate_process_window(&wizard.process) {
                Ok(()) => {
                    app.status = format!("已切换到 {}，请键入 nihao 测试候选栏。", wizard.process)
                }
                Err(err) => app.status = err,
            }
        }
        Some(GameTestWizardAction::CandidateVisible) => {
            if let Some(state) = app.game_test_wizard.as_mut() {
                state.step = GameTestStep::Position;
            }
        }
        Some(GameTestWizardAction::CandidateMissing) => {
            app.set_overlay_backend_for_process(&wizard.process, "external");
            if app.save().is_ok() {
                app.status = format!(
                    "已为 {} 改用独立 Overlay；切回游戏后再键入 nihao 测试。",
                    wizard.process
                );
            }
        }
        Some(GameTestWizardAction::PositionGood) => {
            if let Some(state) = app.game_test_wizard.as_mut() {
                state.step = GameTestStep::Commit;
            }
        }
        Some(GameTestWizardAction::AdjustPosition) => {
            app.active_section = SettingsSection::Compatibility;
            app.reset_section_scroll = true;
            app.game_test_wizard = None;
            app.status = format!(
                "请在 {} 的“候选 Overlay”中调整位置后重新测试。",
                wizard.process
            );
        }
        Some(GameTestWizardAction::CommitGood) => {
            app.game_test_wizard = None;
            app.status = format!("{} 的游戏候选栏测试已完成。", wizard.process);
        }
        Some(GameTestWizardAction::UseUnicode) => {
            app.set_commit_transport_for_process(&wizard.process, "unicode_sendinput");
            if app.save().is_ok() {
                app.status = format!(
                    "已为 {} 改用 Unicode SendInput，请回到游戏重试。",
                    wizard.process
                );
            }
        }
        Some(GameTestWizardAction::UseClipboard) => {
            app.set_commit_transport_for_process(&wizard.process, "clipboard_paste");
            if app.save().is_ok() {
                app.status = format!("已为 {} 改用剪贴板粘贴，请回到游戏重试。", wizard.process);
            }
        }
        Some(GameTestWizardAction::Close) => app.game_test_wizard = None,
        None => {}
    }
}

fn compatibility_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "应用兼容", |ui| {
        setting_toggle(
            ui,
            "检测全屏",
            "在游戏或全屏应用中自动应用兼容策略。",
            &mut app.model.fullscreen_detection,
        );
        setting_combo_row(
            ui,
            "全屏策略",
            "进入全屏时如何处理输入法状态和界面。",
            fullscreen_policy_label(&app.model.fullscreen_policy),
            "fullscreen_policy",
            |ui| {
                selectable_string(
                    ui,
                    &mut app.model.fullscreen_policy,
                    schema_options::FULLSCREEN_POLICIES[0],
                    "显示候选栏",
                );
                selectable_string(
                    ui,
                    &mut app.model.fullscreen_policy,
                    schema_options::FULLSCREEN_POLICIES[1],
                    "英文模式",
                );
                selectable_string(
                    ui,
                    &mut app.model.fullscreen_policy,
                    schema_options::FULLSCREEN_POLICIES[2],
                    "隐藏界面",
                );
                selectable_string(
                    ui,
                    &mut app.model.fullscreen_policy,
                    schema_options::FULLSCREEN_POLICIES[3],
                    "关闭",
                );
            },
        );
        setting_combo_row(
            ui,
            "上屏方式",
            "自动模式会在游戏或全屏兼容档中优先使用 Unicode，上屏失败时也会尝试降级。",
            commit_transport_label(&app.model.commit_transport),
            "commit_transport",
            |ui| {
                selectable_string(
                    ui,
                    &mut app.model.commit_transport,
                    schema_options::COMMIT_TRANSPORTS[0],
                    "自动",
                );
                selectable_string(
                    ui,
                    &mut app.model.commit_transport,
                    schema_options::COMMIT_TRANSPORTS[1],
                    "标准 TSF",
                );
                selectable_string(
                    ui,
                    &mut app.model.commit_transport,
                    schema_options::COMMIT_TRANSPORTS[2],
                    "剪贴板粘贴",
                );
                selectable_string(
                    ui,
                    &mut app.model.commit_transport,
                    schema_options::COMMIT_TRANSPORTS[3],
                    "Unicode SendInput",
                );
            },
        );
        setting_toggle(
            ui,
            "内置游戏进程列表",
            "自动识别常见游戏和全屏应用。",
            &mut app.model.builtin_game_list,
        );
        setting_toggle(
            ui,
            "自动建议应用规则",
            "根据运行中的应用建议兼容性配置。",
            &mut app.model.auto_suggest_app_options,
        );
        ui.separator();
        compatibility_match_status_ui(ui, app);
        ui.separator();
        compatibility_recent_logs_ui(ui);
        ui.separator();
        compatibility_rules_ui(ui, app);
        ui.separator();
        recent_process_suggestions_ui(ui, app);
    });
}

fn compatibility_match_status_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("当前前台应用")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        if outline_button(ui, "刷新").clicked() {
            app.foreground_process = current_foreground_process();
            app.refresh_recent_processes();
        }
    });
    let Some(foreground) = app.foreground_process.clone() else {
        inline_notice(
            ui,
            StatusTone::Info,
            "未获取到前台窗口。可点击“刷新”重新检测。",
        );
        return;
    };
    let title = if foreground.title.trim().is_empty() {
        "无窗口标题".to_string()
    } else {
        foreground.title.clone()
    };
    let match_text = matching_compat_rule(&app.model.compat_rules, &foreground.name)
        .map(|rule| {
            if rule.enabled {
                let mut text = rule.policy.label().to_string();
                let transport = normalize_commit_transport_value(&rule.commit_transport, "global");
                if transport != "global" {
                    text.push_str(" / ");
                    text.push_str(commit_transport_label(&transport));
                }
                if rule.game_profile {
                    text.push_str(" / 游戏档");
                }
                text
            } else {
                "规则已停用".to_string()
            }
        })
        .unwrap_or_else(|| "未命中自定义规则".to_string());
    ui.label(format!("{}  -  {}", foreground.name, title));
    ui.label(
        RichText::new(format!("命中规则：{match_text}"))
            .small()
            .color(palette.muted),
    );
    let recommended = recommended_compat_policy_for_process(&foreground);
    let likely_game = is_likely_game_process(&foreground);
    let recommended_text = if likely_game {
        "显示候选栏 / 紧凑游戏档 / 标准 TSF 优先 / 自动 Overlay".to_string()
    } else {
        recommended.label().to_string()
    };
    ui.label(
        RichText::new(format!("推荐策略：{recommended_text}"))
            .small()
            .color(palette.muted),
    );
    ui.horizontal_wrapped(|ui| {
        if outline_button(ui, "隐藏候选").clicked() {
            app.set_compat_policy_for_process(&foreground.name, CompatRulePolicy::HideUi);
        }
        if outline_button(ui, "强制英文").clicked() {
            app.set_compat_policy_for_process(&foreground.name, CompatRulePolicy::Ascii);
        }
        if outline_button(ui, "显示候选").clicked() {
            app.set_compat_policy_for_process(&foreground.name, CompatRulePolicy::ShowUi);
        }
        if outline_button(ui, "游戏档").clicked() {
            app.set_game_profile_for_process(&foreground.name);
        }
        if outline_button(ui, "应用推荐").clicked() {
            if likely_game {
                app.set_game_profile_for_process(&foreground.name);
            } else {
                app.set_compat_policy_for_process(&foreground.name, recommended);
            }
        }
        if outline_button(ui, "测试该游戏").clicked() {
            app.open_game_test_wizard(&foreground.name, &foreground.title);
        }
    });
}

pub(super) fn is_likely_game_process(process: &ProcessSuggestion) -> bool {
    let name = process.name.to_ascii_lowercase();
    let title = process.title.to_ascii_lowercase();
    process.fullscreen
        || name.contains("game")
        || name.contains("steam")
        || name.contains("unity")
        || name.contains("unreal")
        || title.contains("fullscreen")
}

pub(super) fn recommended_compat_policy_for_process(
    process: &ProcessSuggestion,
) -> CompatRulePolicy {
    if is_likely_game_process(process) {
        CompatRulePolicy::ShowUi
    } else {
        CompatRulePolicy::Global
    }
}

fn compatibility_recent_logs_ui(ui: &mut egui::Ui) {
    let palette = fluent_palette(ui);
    ui.label(
        RichText::new("最近兼容/降级日志")
            .strong()
            .size(SETTINGS_FONT_SECTION_TITLE),
    );
    let lines = recent_compatibility_log_lines(8);
    if lines.is_empty() {
        ui.label(
            RichText::new("暂无兼容降级记录。")
                .small()
                .color(palette.muted),
        );
        return;
    }
    egui::Frame::none()
        .fill(palette.surface_alt)
        .stroke(Stroke::new(1.0, palette.border_subtle))
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            for line in lines {
                ui.label(RichText::new(line).monospace().small().color(palette.text));
            }
        });
}

fn compatibility_rules_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("游戏进程匹配列表")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        if outline_button(ui, "添加空规则").clicked() {
            app.model.compat_rules.push(CompatRule {
                enabled: true,
                process: String::new(),
                policy: CompatRulePolicy::ShowUi,
                commit_transport: "auto".to_string(),
                game_profile: true,
                overlay_anchor: schema_default::OVERLAY_ANCHOR.to_string(),
                overlay_offset_x: 0,
                overlay_offset_y: 0,
                overlay_scale_percent: schema_default::OVERLAY_SCALE_PERCENT,
                overlay_monitor: schema_default::OVERLAY_MONITOR.to_string(),
                overlay_backend: schema_default::OVERLAY_BACKEND.to_string(),
            });
        }
        if let Some(foreground) = app.foreground_process.clone() {
            if outline_button(ui, "添加当前应用").clicked() {
                app.add_compat_process(&foreground.name);
            }
        }
    });

    if app.model.compat_rules.is_empty() {
        ui.label(
            RichText::new("尚未添加自定义兼容规则。")
                .small()
                .color(palette.muted),
        );
        return;
    }

    let mut remove_index = None;
    let mut test_process = None;
    egui::Grid::new("compat_rules_grid")
        .num_columns(6)
        .striped(true)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.strong("启用");
            ui.strong("进程名");
            ui.strong("策略");
            ui.strong("上屏方式");
            ui.strong("游戏档");
            ui.strong("操作");
            ui.end_row();

            for (idx, rule) in app.model.compat_rules.iter_mut().enumerate() {
                ui.checkbox(&mut rule.enabled, "");
                ui.add(TextEdit::singleline(&mut rule.process).desired_width(180.0));
                ComboBox::from_id_salt(("compat_policy", idx))
                    .selected_text(rule.policy.label())
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for policy in CompatRulePolicy::ALL {
                            ui.selectable_value(&mut rule.policy, policy, policy.label());
                        }
                    });
                rule.commit_transport =
                    normalize_commit_transport_value(&rule.commit_transport, "global");
                ComboBox::from_id_salt(("compat_commit_transport", idx))
                    .selected_text(commit_transport_label(&rule.commit_transport))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut rule.commit_transport,
                            "global".to_string(),
                            "使用全局方式",
                        );
                        for transport in schema_options::COMMIT_TRANSPORTS {
                            ui.selectable_value(
                                &mut rule.commit_transport,
                                (*transport).to_string(),
                                commit_transport_label(transport),
                            );
                        }
                    });
                ui.checkbox(&mut rule.game_profile, "");
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!rule.process.trim().is_empty(), egui::Button::new("测试"))
                        .clicked()
                    {
                        test_process = Some(rule.process.clone());
                    }
                    if danger_button(ui, "删除").clicked() {
                        remove_index = Some(idx);
                    }
                });
                ui.end_row();
            }
        });

    if let Some(idx) = remove_index {
        app.model.compat_rules.remove(idx);
    }
    if !app.model.compat_rules.is_empty() {
        ui.add_space(10.0);
        ui.label(
            RichText::new("每游戏候选 Overlay")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        ui.label(
            RichText::new(
                "位置偏移使用逻辑像素；自动后端会优先选择适合游戏的 Overlay，并在失败时回退。",
            )
            .small()
            .color(palette.muted),
        );

        for (idx, rule) in app.model.compat_rules.iter_mut().enumerate() {
            let process_label = if rule.process.trim().is_empty() {
                format!("未命名规则 {}", idx + 1)
            } else {
                rule.process.trim().to_string()
            };
            rule.overlay_anchor = normalize_overlay_anchor_value(&rule.overlay_anchor);
            rule.overlay_monitor = normalize_overlay_monitor_value(&rule.overlay_monitor);
            rule.overlay_backend = normalize_overlay_backend_value(&rule.overlay_backend);
            rule.overlay_offset_x = rule.overlay_offset_x.clamp(-4000, 4000);
            rule.overlay_offset_y = rule.overlay_offset_y.clamp(-4000, 4000);
            rule.overlay_scale_percent = rule.overlay_scale_percent.clamp(50, 200);

            egui::CollapsingHeader::new(format!(
                "{} · {} · {}%",
                process_label,
                overlay_anchor_label(&rule.overlay_anchor),
                rule.overlay_scale_percent
            ))
            .id_salt(("compat_overlay", idx))
            .show(ui, |ui| {
                egui::Grid::new(("compat_overlay_grid", idx))
                    .num_columns(2)
                    .spacing([18.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("锚点");
                        ComboBox::from_id_salt(("compat_overlay_anchor", idx))
                            .selected_text(overlay_anchor_label(&rule.overlay_anchor))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for anchor in schema_options::OVERLAY_ANCHORS {
                                    ui.selectable_value(
                                        &mut rule.overlay_anchor,
                                        (*anchor).to_string(),
                                        overlay_anchor_label(anchor),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("位置偏移 X / Y");
                        ui.horizontal(|ui| {
                            ui.add(
                                Slider::new(&mut rule.overlay_offset_x, -4000..=4000)
                                    .suffix(" px")
                                    .text("X"),
                            );
                            ui.add(
                                Slider::new(&mut rule.overlay_offset_y, -4000..=4000)
                                    .suffix(" px")
                                    .text("Y"),
                            );
                        });
                        ui.end_row();

                        ui.label("缩放");
                        ui.add(Slider::new(&mut rule.overlay_scale_percent, 50..=200).suffix("%"));
                        ui.end_row();

                        ui.label("目标显示器");
                        ComboBox::from_id_salt(("compat_overlay_monitor", idx))
                            .selected_text(overlay_monitor_label(&rule.overlay_monitor))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut rule.overlay_monitor,
                                    "auto".to_string(),
                                    "游戏所在显示器（推荐）",
                                );
                                ui.selectable_value(
                                    &mut rule.overlay_monitor,
                                    "primary".to_string(),
                                    "主显示器",
                                );
                                for monitor_index in 0..8 {
                                    ui.selectable_value(
                                        &mut rule.overlay_monitor,
                                        monitor_index.to_string(),
                                        format!("显示器 {}", monitor_index + 1),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("Overlay 后端");
                        ComboBox::from_id_salt(("compat_overlay_backend", idx))
                            .selected_text(overlay_backend_label(&rule.overlay_backend))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for backend in schema_options::OVERLAY_BACKENDS {
                                    ui.selectable_value(
                                        &mut rule.overlay_backend,
                                        (*backend).to_string(),
                                        overlay_backend_label(backend),
                                    );
                                }
                            });
                        ui.end_row();
                        ui.label("");
                        ui.label(
                            RichText::new(
                                "自动：窗口化时走低延迟进程内候选，全屏或 UI-less 时切换独立 Overlay；“始终使用”用于强制排障。",
                            )
                            .small()
                            .color(palette.muted),
                        );
                        ui.end_row();
                    });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("恢复游戏推荐").clicked() {
                        rule.overlay_anchor = schema_default::OVERLAY_ANCHOR.to_string();
                        rule.overlay_offset_x = 0;
                        rule.overlay_offset_y = 0;
                        rule.overlay_scale_percent = schema_default::OVERLAY_SCALE_PERCENT;
                        rule.overlay_monitor = schema_default::OVERLAY_MONITOR.to_string();
                        rule.overlay_backend = schema_default::OVERLAY_BACKEND.to_string();
                    }
                    if ui
                        .add_enabled(
                            !rule.process.trim().is_empty(),
                            egui::Button::new("测试该游戏"),
                        )
                        .clicked()
                    {
                        test_process = Some(rule.process.clone());
                    }
                });
            });
        }
    }
    sync_compat_rules_to_legacy_fields(&mut app.model);
    if let Some(process) = test_process {
        let title = app
            .recent_processes
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&process))
            .map(|item| item.title.clone())
            .unwrap_or_default();
        app.open_game_test_wizard(&process, &title);
    }
}

fn recent_process_suggestions_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("最近进程推荐")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        if outline_button(ui, "刷新").clicked() {
            app.refresh_recent_processes();
        }
    });
    ui.horizontal_wrapped(|ui| {
        for process in app.recent_processes.clone() {
            let mut label = format!("+ {}", process.name);
            if process.foreground {
                label.push_str(" 前台");
            }
            if process.fullscreen {
                label.push_str(" 全屏");
            }
            let response = ui.small_button(label);
            let response = if process.title.trim().is_empty() {
                response
            } else {
                response.on_hover_text(process.title.clone())
            };
            if response.clicked() {
                app.add_compat_process(&process.name);
            }
        }
    });
    ui.label(
        RichText::new("优先显示前台窗口和带标题的应用，已过滤系统后台进程。")
            .small()
            .color(palette.muted),
    );
}

fn screenshot_settings_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
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

fn privacy_data_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
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

fn privacy_process_list_row(ui: &mut egui::Ui, title: &str, description: &str, value: &mut String) {
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

fn data_location_row(
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

fn folder_path_row(
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

fn executable_path_row(
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

fn filename_pattern_row(
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

fn diagnostic_log_dir() -> PathBuf {
    runtime_log::log_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn compatibility_log_path() -> Option<PathBuf> {
    app_paths::local_data_dir().map(|dir| dir.join("compatibility.log"))
}

pub(super) fn diagnostic_log_paths() -> Vec<PathBuf> {
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

fn clipboard_storage_status_text(path: &Path) -> String {
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

pub(super) fn privacy_statement_text(config_path: &Path, model: &SettingsModel) -> String {
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

fn input_experience_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    input_habits_ui(ui, &mut app.model);
    ui.add_space(2.0);
    input_tools_ui(ui, app);
}

fn candidate_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp, request_real_preview: &mut bool) {
    candidate_appearance_ui(
        ui,
        &mut app.model,
        &mut app.status,
        &app.available_skins,
        &app.available_chinese_fonts,
        request_real_preview,
    );
}

fn lexicon_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    lexicon_learning_ui(ui, app);
    ui.add_space(2.0);
    custom_shortcuts_ui(ui, app);
}

fn screenshot_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    screenshot_settings_ui(ui, app);
}

fn diagnostics_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    diagnostics_ui(ui, app);
}

fn ocr_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
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

fn translation_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
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

fn advanced_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    input_experience_ui(ui, app);
    ui.add_space(2.0);
    advanced_rank_ui(ui, app);
    ui.add_space(2.0);
    advanced_engine_tuning_ui(ui, app);
}

fn settings_collapsing_section(
    ui: &mut egui::Ui,
    title: &str,
    id: &'static str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let palette = fluent_palette(ui);
    egui::CollapsingHeader::new(
        RichText::new(title)
            .strong()
            .size(SETTINGS_FONT_SETTING_TITLE)
            .color(palette.text),
    )
    .id_salt(id)
    .default_open(false)
    .show(ui, |ui| {
        ui.add_space(4.0);
        add_contents(ui);
    });
}

fn advanced_rank_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    settings_collapsing_section(ui, "候选排序权重", "candidate_ranking_section", |ui| {
        section_panel(ui, "候选排序高级参数", |ui| {
            if outline_button(ui, "恢复排序推荐").clicked() {
                app.reset_rank_defaults();
            }
            setting_slider_f64(
                ui,
                "单字权重",
                "单字语言模型对排序的影响。",
                &mut app.model.w_single_lm,
                0.0..=1.0,
            );
            setting_slider_f64(
                ui,
                "词组权重",
                "词组路径得分对排序的影响。",
                &mut app.model.w_phrase_path,
                0.0..=1.0,
            );
            setting_slider_f64(
                ui,
                "单字缩放",
                "放大或压低单字语言模型分值。",
                &mut app.model.lm_single_scale,
                0.1..=30.0,
            );
        });
    });
}

fn advanced_engine_tuning_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    settings_collapsing_section(ui, "引擎与缓存调优", "engine_tuning_section", |ui| {
        section_panel(ui, "候选首屏延迟参数", |ui| {
            if outline_button(ui, "恢复性能推荐").clicked() {
                app.reset_engine_tuning_defaults();
            }
            ui.label(
                RichText::new(
                    "长输入先返回首屏候选；纠错和完整重排只在首屏后继续，避免连续输入被慢路径阻塞。",
                )
                .small()
                .color(fluent_palette(ui).muted),
            );
            setting_slider_usize(
                ui,
                "前缀缓存容量",
                "影响连续输入时前缀词库查询的复用数量。",
                &mut app.model.prefix_cache_capacity,
                8..=512,
            );
            setting_slider_usize(
                ui,
                "最终查询缓存容量",
                "缓存完整候选基础结果；选择反馈会在命中后重新轻量应用。",
                &mut app.model.final_lookup_cache_capacity,
                8..=512,
            );
            setting_slider_usize(
                ui,
                "短输入缓存容量",
                "缓存短拼音/简拼输入结果。",
                &mut app.model.short_lookup_cache_capacity,
                8..=512,
            );
            setting_slider_usize(
                ui,
                "长输入软预算 ms",
                "首屏候选的软时间预算；4 ms 为速度优先，调高会减少部分结果但增加首屏等待。",
                &mut app.model.long_lookup_soft_budget_ms,
                1..=50,
            );
            setting_slider_usize(
                ui,
                "首批候选下限",
                "达到这个数量后才触发首屏截止；速度优先可设为 6。",
                &mut app.model.long_lookup_min_first_batch_candidates,
                1..=128,
            );
        });
    });
}

fn recent_perf_log_lines(limit: usize) -> Vec<String> {
    let patterns = [
        "srf_engine_load",
        "srf_engine_lexicon_mode",
        "srf_engine_ensure_loaded",
        "srf_engine_full_warmup",
        "engine_helper_start",
        "srf_ipc_lookup",
        "srf_lookup_profile",
        "candidate-refresh",
    ];
    let mut lines = runtime_log::recent_lines_matching(limit, &patterns);
    if lines.len() >= limit {
        return lines;
    }
    for path in diagnostic_log_paths() {
        let text = fs::read_to_string(&path).unwrap_or_default();
        lines.extend(
            text.lines()
                .filter(|line| patterns.iter().any(|pattern| line.contains(pattern)))
                .map(|line| {
                    format!(
                        "{}  {}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        line
                    )
                }),
        );
    }
    if lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    lines
}

fn recent_compatibility_log_lines(limit: usize) -> Vec<String> {
    let patterns = [
        "compat",
        "fullscreen",
        "fallback",
        "candidateui",
        "candidate ui",
    ];
    let mut lines = runtime_log::recent_lines_matching(limit, &patterns);
    if lines.len() >= limit {
        return lines;
    }
    for path in diagnostic_log_paths() {
        let text = fs::read_to_string(&path).unwrap_or_default();
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        for line in text.lines() {
            let lower = line.to_ascii_lowercase();
            let is_compat_log = file_name.eq_ignore_ascii_case("compatibility.log");
            if is_compat_log || patterns.iter().any(|pattern| lower.contains(pattern)) {
                lines.push(format!("{file_name}  {line}"));
            }
        }
    }
    if lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    lines
}

fn latest_log_line_matching(patterns: &[&str]) -> Option<String> {
    let mut found = None;
    for path in diagnostic_log_paths() {
        let text = fs::read_to_string(&path).unwrap_or_default();
        for line in text.lines() {
            if patterns.iter().any(|pattern| line.contains(pattern)) {
                found = Some(format!(
                    "{}  {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    line
                ));
            }
        }
    }
    found
}

fn compact_diagnostic_line(line: &str, max_chars: usize) -> String {
    let compact = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let value = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{value}...")
    } else {
        value
    }
}

fn cold_start_summary_lines() -> Vec<String> {
    let probes: [(&str, &[&str]); 5] = [
        ("tray", &["engine_helper_start"]),
        ("engine", &["srf_engine_lexicon_mode"]),
        ("hot/full", &["srf_engine_full_warmup_finish"]),
        ("first lookup", &["srf_ipc_lookup"][..]),
        ("ensure", &["srf_engine_ensure_loaded"]),
    ];
    probes
        .iter()
        .filter_map(|(label, patterns)| {
            latest_log_line_matching(patterns).map(|line| format!("{label}: {line}"))
        })
        .collect()
}

#[derive(Clone)]
struct LatencyStatsRow {
    label: &'static str,
    count: usize,
    p50_ms: f64,
    p90_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

fn metric_token_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest
        .find(|ch: char| ch.is_ascii_whitespace() || ch == ',' || ch == ';')
        .unwrap_or(rest.len());
    let token = rest[..end].trim();
    (!token.is_empty()).then_some(token)
}

fn metric_number_after(line: &str, key: &str) -> Option<f64> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let mut end = 0usize;
    for (idx, ch) in rest.char_indices() {
        if ch.is_ascii_digit() || ch == '.' {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    (end > 0).then(|| rest[..end].parse::<f64>().ok()).flatten()
}

fn push_latency_sample(
    series: &mut BTreeMap<&'static str, Vec<f64>>,
    label: &'static str,
    value_ms: f64,
) {
    if value_ms.is_finite() && value_ms >= 0.0 {
        series.entry(label).or_default().push(value_ms);
    }
}

fn collect_latency_line(series: &mut BTreeMap<&'static str, Vec<f64>>, line: &str) {
    if line.contains("[perf]") {
        if let (Some(stage), Some(elapsed_ms)) = (
            metric_token_after(line, "stage="),
            metric_number_after(line, "elapsed_ms="),
        ) {
            let label = match stage {
                "Key/WouldEat" => Some("按键预判"),
                "Key/ProcessKey" => Some("按键处理"),
                "CandidateWorker/lookup" => Some("候选查询(worker)"),
                "CandidateWorker/request-to-apply" => Some("按键到候选应用"),
                "CandidateWindow/prepare-resources" => Some("候选窗资源准备"),
                "CandidateWindow/begin-or-update" => Some("候选窗更新"),
                "CandidateWindow/total" => Some("候选窗绘制总计"),
                "CommitCandidate/text-write" => Some("候选上屏写入"),
                _ => None,
            };
            if let Some(label) = label {
                push_latency_sample(series, label, elapsed_ms);
            }
        }
    }

    if line.contains("event=srf_ipc_lookup ") {
        if let Some(total_us) = metric_number_after(line, "total=") {
            push_latency_sample(series, "IPC 查询总计", total_us / 1000.0);
        }
        if let Some(engine_us) = metric_number_after(line, "engine=") {
            push_latency_sample(series, "IPC 引擎内部", engine_us / 1000.0);
        }
    }
    if line.contains("event=srf_ipc_lookup_waited") || line.contains("event=srf_ipc_lookup_busy") {
        if let Some(waited_us) = metric_number_after(line, "waited_us=") {
            push_latency_sample(series, "共享引擎等待", waited_us / 1000.0);
        }
    }

    if line.contains("event=srf_lookup_profile") {
        if let Some(total_us) = metric_number_after(line, "total=") {
            push_latency_sample(series, "Rust 查询总计", total_us / 1000.0);
        }
        for (key, label) in [
            ("prepare=", "Rust 准备"),
            ("decode=", "Rust 解码"),
            ("correction=", "Rust 纠错"),
            ("rerank_sort=", "Rust 排序后处理"),
            ("finish=", "Rust 收尾"),
        ] {
            if let Some(value_us) = metric_number_after(line, key) {
                push_latency_sample(series, label, value_us / 1000.0);
            }
        }
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = ((sorted.len() - 1) as f64 * pct).round() as usize;
    sorted[pos.min(sorted.len() - 1)]
}

fn typing_latency_stats() -> Vec<LatencyStatsRow> {
    let mut series: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    for path in diagnostic_log_paths() {
        let text = fs::read_to_string(&path).unwrap_or_default();
        for line in text.lines() {
            collect_latency_line(&mut series, line);
        }
    }

    let order = [
        "按键预判",
        "按键处理",
        "按键到候选应用",
        "候选查询(worker)",
        "IPC 查询总计",
        "IPC 引擎内部",
        "Rust 查询总计",
        "Rust 准备",
        "Rust 解码",
        "Rust 纠错",
        "Rust 排序后处理",
        "Rust 收尾",
        "候选窗资源准备",
        "候选窗更新",
        "候选窗绘制总计",
        "候选上屏写入",
        "共享引擎等待",
    ];

    let mut rows = Vec::new();
    for label in order {
        let Some(mut values) = series.remove(label) else {
            continue;
        };
        values.sort_by(|a, b| a.total_cmp(b));
        let count = values.len();
        rows.push(LatencyStatsRow {
            label,
            count,
            p50_ms: percentile(&values, 0.50),
            p90_ms: percentile(&values, 0.90),
            p99_ms: percentile(&values, 0.99),
            max_ms: values.last().copied().unwrap_or_default(),
        });
    }
    rows
}

fn format_latency_ms(value: f64) -> String {
    if value < 1.0 {
        format!("{value:.2}")
    } else if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}

fn diagnostic_sqlite_io_error(err: rusqlite::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, err)
}

fn write_typing_latency_summary_sqlite(
    path: &Path,
    rows: &[LatencyStatsRow],
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut conn = rusqlite::Connection::open(path).map_err(diagnostic_sqlite_io_error)?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS typing_latency_summary (
           label TEXT PRIMARY KEY,
           count INTEGER NOT NULL,
           p50_ms REAL NOT NULL,
           p90_ms REAL NOT NULL,
           p99_ms REAL NOT NULL,
           max_ms REAL NOT NULL
         );",
    )
    .map_err(diagnostic_sqlite_io_error)?;
    let tx = conn.transaction().map_err(diagnostic_sqlite_io_error)?;
    tx.execute("DELETE FROM typing_latency_summary", [])
        .map_err(diagnostic_sqlite_io_error)?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO typing_latency_summary
                 (label, count, p50_ms, p90_ms, p99_ms, max_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(diagnostic_sqlite_io_error)?;
        for row in rows {
            stmt.execute(rusqlite::params![
                redact_diagnostic_text(row.label),
                row.count as i64,
                row.p50_ms,
                row.p90_ms,
                row.p99_ms,
                row.max_ms
            ])
            .map_err(diagnostic_sqlite_io_error)?;
        }
    }
    tx.commit().map_err(diagnostic_sqlite_io_error)
}

fn file_modified_summary(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| {
            let local: chrono::DateTime<chrono::Local> = modified.into();
            local.to_rfc3339()
        })
        .unwrap_or_else(|_| "none".to_string())
}

fn diagnostic_redaction_values() -> Vec<(String, &'static str)> {
    let mut values = Vec::new();
    let mut push_value = |value: String, token: &'static str| {
        let trimmed = value.trim().to_string();
        if !trimmed.is_empty() && !values.iter().any(|(existing, _)| existing == &trimmed) {
            values.push((trimmed, token));
        }
    };
    if let Some(path) = app_paths::local_data_dir() {
        push_value(path.display().to_string(), "<DATA_DIR>");
    }
    for (name, token) in [
        ("USERPROFILE", "<USERPROFILE>"),
        ("LOCALAPPDATA", "<LOCALAPPDATA>"),
        ("APPDATA", "<APPDATA>"),
        ("TEMP", "<TEMP>"),
        ("TMP", "<TEMP>"),
        ("COMPUTERNAME", "<COMPUTERNAME>"),
        ("USERNAME", "<USERNAME>"),
    ] {
        if let Ok(value) = std::env::var(name) {
            push_value(value, token);
        }
    }
    values.sort_by(|left, right| right.0.len().cmp(&left.0.len()));
    values
}

fn redact_diagnostic_text(text: &str) -> String {
    let mut redacted = text.to_string();
    for (value, token) in diagnostic_redaction_values() {
        redacted = redacted.replace(&value, token);
    }
    redacted
}

fn append_clipboard_store_summary(summary: &mut String) {
    let path = pinyin_ime::clipboard_store::store_path();
    let exists = path.is_file();
    let bytes = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    summary.push_str("clipboard_store_path=<DATA_DIR>\\clipboard_store.sqlite\n");
    summary.push_str(&format!("clipboard_store_exists={}\n", exists));
    summary.push_str(&format!("clipboard_store_bytes={}\n", bytes));
    summary.push_str(&format!(
        "clipboard_store_modified={}\n",
        file_modified_summary(&path)
    ));
    match pinyin_ime::clipboard_store::snapshot() {
        Ok(snapshot) => {
            summary.push_str(&format!(
                "clipboard_history_count={}\nclipboard_pinned_count={}\n",
                snapshot.history.len(),
                snapshot.pinned.len()
            ));
        }
        Err(err) => {
            summary.push_str(&format!("clipboard_snapshot_error={err}\n"));
        }
    }
}

fn process_running_summary(name: &str) -> String {
    #[cfg(windows)]
    {
        let filter = format!("IMAGENAME eq {name}");
        let output = Command::new("tasklist")
            .arg("/FI")
            .arg(&filter)
            .arg("/NH")
            .output();
        return match output {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
                if text.contains(&name.to_ascii_lowercase()) {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            Err(err) => format!("unknown:{err}"),
        };
    }

    #[cfg(not(windows))]
    {
        let _ = name;
        "unknown".to_string()
    }
}

fn recent_runtime_event_lines(limit: usize) -> Vec<String> {
    let sqlite_lines = runtime_log::recent_event_lines(limit);
    if sqlite_lines.len() >= limit {
        return sqlite_lines;
    }
    let mut lines: Vec<(String, String)> = sqlite_lines
        .into_iter()
        .map(|line| (line.clone(), line))
        .collect();
    for path in runtime_log::current_log_paths() {
        let label = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "runtime.log".to_string());
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines().rev().take(limit) {
            let clipped: String = line.chars().take(2000).collect();
            lines.push((line.to_string(), format!("{label}\t{clipped}")));
        }
    }
    lines.sort_by(|left, right| left.0.cmp(&right.0));
    let selected = if lines.len() > limit {
        lines.split_off(lines.len() - limit)
    } else {
        lines
    };
    selected.into_iter().map(|(_, line)| line).collect()
}

pub(super) fn export_diagnostic_package_to(
    dest: &Path,
    config_path: &Path,
    model: &SettingsModel,
) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    fs::create_dir_all(dest.join("logs"))?;

    let mut summary = String::new();
    summary.push_str("Kaixin IME diagnostics\n");
    summary.push_str(&format!("created={}\n", chrono::Local::now().to_rfc3339()));
    summary.push_str(&format!("version={}\n", env!("CARGO_PKG_VERSION")));
    summary.push_str("config_path=<DATA_DIR>\\kaixin.ini\n");
    summary.push_str(&format!("config_exists={}\n", config_path.is_file()));
    summary.push_str(&format!(
        "config_modified={}\n",
        file_modified_summary(config_path)
    ));
    summary.push_str("data_dir=<DATA_DIR>\\logs\n");
    summary.push_str(&format!("log_level={}\n", model.log_level.trim()));
    summary.push_str(&format!(
        "clipboard_background_enabled={}\n",
        model.clipboard_background_enabled
    ));
    summary.push_str(&format!(
        "privacy_never_learn_process_count={}\n",
        model.privacy_never_learn_processes.len()
    ));
    summary.push_str(&format!(
        "privacy_never_clipboard_process_count={}\n",
        model.privacy_never_clipboard_processes.len()
    ));
    summary.push_str(&format!(
        "privacy_never_candidate_process_count={}\n",
        model.privacy_never_candidate_processes.len()
    ));
    summary.push_str(&format!("mixed_pinyin={}\n", model.mixed_pinyin));
    append_clipboard_store_summary(&mut summary);
    summary.push_str(&format!(
        "process_srf_ime_tray_running={}\n",
        process_running_summary("srf_ime_tray.exe")
    ));
    summary.push_str(&format!(
        "process_srf_ime_engine_running={}\n",
        process_running_summary("srf_ime_engine.exe")
    ));
    summary.push_str("\nlogs:\n");

    for path in diagnostic_log_paths() {
        let exists = path.is_file();
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        summary.push_str(&format!(
            "- {} exists={} bytes={}\n",
            redact_diagnostic_text(&path.display().to_string()),
            exists,
            size
        ));
        if exists {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "log.txt".to_string());
            let mut dest_name = file_name.clone();
            let mut idx = 2usize;
            while dest.join("logs").join(&dest_name).exists() {
                dest_name = format!("{idx}-{file_name}");
                idx += 1;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                let _ = fs::write(
                    dest.join("logs").join(dest_name),
                    redact_diagnostic_text(&text),
                );
            }
        }
    }

    fs::write(dest.join("summary.txt"), redact_diagnostic_text(&summary))?;
    let recent_events = recent_runtime_event_lines(20).join("\n");
    fs::write(
        dest.join("recent-events.log"),
        redact_diagnostic_text(&recent_events),
    )?;
    let recent = recent_perf_log_lines(80).join("\n");
    fs::write(
        dest.join("recent-perf.log"),
        redact_diagnostic_text(&recent),
    )?;
    let compat = recent_compatibility_log_lines(80).join("\n");
    fs::write(
        dest.join("recent-compatibility.log"),
        redact_diagnostic_text(&compat),
    )?;
    let latency_rows = typing_latency_stats();
    write_typing_latency_summary_sqlite(
        &dest.join("typing-latency-summary.sqlite"),
        &latency_rows,
    )?;
    Ok(())
}

fn log_level_label(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => "关闭",
        "error" => "错误",
        "perf" => "性能",
        "verbose" => "详细",
        _ => "基础",
    }
}

fn learning_sensitivity_label(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "conservative" => "保守",
        "aggressive" => "积极",
        _ => "标准",
    }
}

fn user_hotword_boost_label(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "conservative" => "保守",
        "strong" => "强",
        "aggressive" => "积极",
        _ => "标准",
    }
}

fn engine_recovery_state_summary() -> Option<String> {
    let reason = read_state_string("LastEngineRecoveryReason")?;
    let time =
        read_state_string("LastEngineRecoveryTime").unwrap_or_else(|| "时间未知".to_string());
    Some(format!("{time}  {reason}"))
}

#[cfg(windows)]
fn read_state_string(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let subkey: Vec<u16> = r"Software\kaixin\State"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != 0 || bytes <= 2 {
        return None;
    }

    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    if let Some(pos) = buffer.iter().position(|unit| *unit == 0) {
        buffer.truncate(pos);
    }
    let text = String::from_utf16_lossy(&buffer).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(not(windows))]
fn read_state_string(_name: &str) -> Option<String> {
    None
}

#[derive(Clone)]
pub(super) struct DiagnosticsSnapshot {
    pub(super) refreshed_at: Instant,
    recovery: Option<String>,
    cold_lines: Vec<String>,
    recent_lines: Vec<String>,
    compat_lines: Vec<String>,
    latency_rows: Vec<LatencyStatsRow>,
    foreground: Option<ProcessSuggestion>,
    latest_candidate_refresh: Option<String>,
}

pub(super) fn build_diagnostics_snapshot(app: &SettingsApp) -> DiagnosticsSnapshot {
    DiagnosticsSnapshot {
        refreshed_at: Instant::now(),
        recovery: engine_recovery_state_summary(),
        cold_lines: cold_start_summary_lines(),
        recent_lines: recent_perf_log_lines(12),
        compat_lines: recent_compatibility_log_lines(12),
        latency_rows: typing_latency_stats(),
        foreground: app.foreground_process.clone(),
        latest_candidate_refresh: latest_log_line_matching(&["candidate-refresh"]),
    }
}

fn diagnostics_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let Some(snapshot) = app.diagnostics_cache.as_ref().cloned() else {
        ui.label("正在读取诊断数据…");
        return;
    };
    let recovery = snapshot.recovery.as_ref();
    let cold_lines = &snapshot.cold_lines;
    let recent_lines = &snapshot.recent_lines;
    let compat_lines = &snapshot.compat_lines;
    let latency_rows = &snapshot.latency_rows;
    let foreground = snapshot.foreground.as_ref();
    let foreground_policy = foreground.and_then(|process| {
        matching_compat_rule(&app.model.compat_rules, &process.name).map(|rule| {
            if rule.enabled {
                rule.policy.label().to_string()
            } else {
                "规则已停用".to_string()
            }
        })
    });
    let latest_candidate_refresh = snapshot.latest_candidate_refresh.as_ref();
    let palette = fluent_palette(ui);
    let engine_value = recovery
        .map(|value| compact_diagnostic_line(value, 52))
        .unwrap_or_else(|| "正常".to_string());
    let foreground_value = foreground
        .map(|process| process.name.clone())
        .unwrap_or_else(|| "未获取".to_string());
    let policy_value = foreground_policy
        .clone()
        .unwrap_or_else(|| "未命中自定义规则".to_string());
    let performance_value = if recent_lines.is_empty() {
        "暂无性能事件".to_string()
    } else {
        format!("最近 {} 条", recent_lines.len())
    };
    let latency_value = if latency_rows.is_empty() {
        "暂无样本".to_string()
    } else {
        format!("{} 项指标", latency_rows.len())
    };
    let refresh_value = latest_candidate_refresh
        .map(String::as_str)
        .map(|line| compact_diagnostic_line(line, 52))
        .unwrap_or_else(|| "暂无刷新耗时".to_string());
    let compat_value = if compat_lines.is_empty() {
        "暂无记录".to_string()
    } else {
        format!("最近 {} 条", compat_lines.len())
    };
    let cold_value = if cold_lines.is_empty() {
        "暂无摘要".to_string()
    } else {
        format!("{} 条摘要", cold_lines.len())
    };
    let status_items = [
        (
            "引擎",
            engine_value.as_str(),
            if recovery.is_some() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "前台进程",
            foreground_value.as_str(),
            if foreground.is_some() {
                palette.success
            } else {
                palette.warning
            },
        ),
        (
            "兼容策略",
            policy_value.as_str(),
            if foreground_policy.is_some() {
                palette.success
            } else {
                palette.warning
            },
        ),
        (
            "性能日志",
            performance_value.as_str(),
            if recent_lines.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "延迟统计",
            latency_value.as_str(),
            if latency_rows.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "候选刷新",
            refresh_value.as_str(),
            if latest_candidate_refresh.is_some() {
                palette.success
            } else {
                palette.warning
            },
        ),
        (
            "兼容降级",
            compat_value.as_str(),
            if compat_lines.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "冷启动",
            cold_value.as_str(),
            if cold_lines.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
    ];
    quiet_section(ui, "运行状态", |ui| {
        ui.columns(2, |columns| {
            for (idx, (label, value, color)) in status_items.iter().enumerate() {
                diagnostic_status_card(&mut columns[idx % 2], label, value, *color);
                columns[idx % 2].add_space(8.0);
            }
        });
    });

    ui.add_space(10.0);
    section_panel(ui, "打字延迟统计", |ui| {
        let palette = fluent_palette(ui);
        if latency_rows.is_empty() {
            ui.label(
                RichText::new("暂无延迟样本。把日志级别临时切到“性能”，正常打字一小段后再回来看。")
                    .small()
                    .color(palette.muted),
            );
        } else {
            egui::Grid::new("typing_latency_stats_grid")
                .num_columns(6)
                .striped(true)
                .spacing([14.0, 7.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("阶段").strong().color(palette.text));
                    ui.label(RichText::new("样本").strong().color(palette.text));
                    ui.label(RichText::new("P50 ms").strong().color(palette.text));
                    ui.label(RichText::new("P90 ms").strong().color(palette.text));
                    ui.label(RichText::new("P99 ms").strong().color(palette.text));
                    ui.label(RichText::new("Max ms").strong().color(palette.text));
                    ui.end_row();
                    for row in latency_rows {
                        ui.label(RichText::new(row.label).color(palette.text));
                        ui.label(RichText::new(row.count.to_string()).monospace());
                        ui.label(RichText::new(format_latency_ms(row.p50_ms)).monospace());
                        ui.label(RichText::new(format_latency_ms(row.p90_ms)).monospace());
                        ui.label(RichText::new(format_latency_ms(row.p99_ms)).monospace());
                        ui.label(RichText::new(format_latency_ms(row.max_ms)).monospace());
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "统计来自最近的 TSF / engine 日志；输入内容已脱敏，只保留耗时和样本数量。",
                )
                .small()
                .color(palette.muted),
            );
        }
    });

    ui.add_space(10.0);
    section_panel(ui, "诊断控制", |ui| {
        let palette = fluent_palette(ui);
        setting_combo_row(
            ui,
            "日志级别",
            "off/error/basic/perf/verbose；默认 basic，性能日志建议排障时临时开启。",
            log_level_label(&app.model.log_level).to_string(),
            "diagnostic_log_level",
            |ui| {
                selectable_string(ui, &mut app.model.log_level, "off", "关闭");
                selectable_string(ui, &mut app.model.log_level, "error", "错误");
                selectable_string(ui, &mut app.model.log_level, "basic", "基础");
                selectable_string(ui, &mut app.model.log_level, "perf", "性能");
                selectable_string(ui, &mut app.model.log_level, "verbose", "详细");
            },
        );
        ui.horizontal(|ui| {
            if outline_button(ui, "打开日志").clicked() {
                app.open_data_location(diagnostic_log_dir());
            }
            if danger_button(ui, "清空日志").clicked() {
                app.clear_tsf_log();
            }
            if outline_button(ui, "导出诊断包").clicked() {
                app.export_diagnostic_package();
            }
        });
        ui.label(
            RichText::new("日志默认脱敏；排障时临时切到“性能”，完成后建议改回“基础”。")
                .small()
                .color(palette.muted),
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "最近事件", |ui| {
        let palette = fluent_palette(ui);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("最近 12 条诊断事件")
                    .size(SETTINGS_FONT_SMALL)
                    .color(palette.muted),
            );
            if recent_lines.is_empty() && compat_lines.is_empty() && cold_lines.is_empty() {
                ui.label(RichText::new("暂无记录").small().color(palette.muted));
            }
        });
        ui.add_space(8.0);
        egui::Frame::none()
            .fill(palette.surface_alt)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("recent_diagnostic_events")
                    .max_height(260.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if !cold_lines.is_empty() {
                            diagnostic_log_group(ui, "冷启动摘要", &cold_lines);
                        }
                        if !compat_lines.is_empty() {
                            diagnostic_log_group(ui, "兼容 / 降级", &compat_lines);
                        }
                        if !recent_lines.is_empty() {
                            diagnostic_log_group(ui, "性能事件", &recent_lines);
                        }
                        if recent_lines.is_empty()
                            && compat_lines.is_empty()
                            && cold_lines.is_empty()
                        {
                            ui.label(RichText::new("暂无性能或兼容事件。").color(palette.muted));
                        }
                    });
            });
    });
}

fn diagnostic_log_group(ui: &mut egui::Ui, title: &str, lines: &[String]) {
    let palette = fluent_palette(ui);
    ui.label(
        RichText::new(title)
            .strong()
            .size(SETTINGS_FONT_SMALL)
            .color(palette.text),
    );
    ui.add_space(4.0);
    for line in lines {
        ui.add(
            egui::Label::new(
                RichText::new(line)
                    .monospace()
                    .size(SETTINGS_FONT_LOG)
                    .color(palette.text),
            )
            .wrap(),
        );
        ui.add_space(2.0);
    }
    ui.add_space(8.0);
}
