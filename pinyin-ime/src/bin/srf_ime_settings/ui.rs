use super::*;

const SETTINGS_FONT_HEADING: f32 = 20.0;
const SETTINGS_FONT_PAGE_TITLE: f32 = 23.0;
const SETTINGS_FONT_BRAND_TITLE: f32 = 17.0;
const SETTINGS_FONT_SECTION_TITLE: f32 = 16.0;
const SETTINGS_FONT_SETTING_TITLE: f32 = 15.0;
const SETTINGS_FONT_NAV_TITLE: f32 = 15.0;
const SETTINGS_FONT_BODY: f32 = 15.0;
const SETTINGS_MIN_HINT_FONT: f32 = 10.0;
const SETTINGS_FONT_SMALL: f32 = 14.0;
const SETTINGS_FONT_MONOSPACE: f32 = 14.0;
const SETTINGS_FONT_LOG: f32 = 12.0;
const SETTINGS_CONTROL_WIDTH: f32 = 320.0;
const SETTINGS_CONTROL_MIN_WIDTH: f32 = 280.0;
const SETTINGS_CONTROL_HEIGHT: f32 = 52.0;
const SETTINGS_ROW_HEIGHT: f32 = 52.0;
const SETTINGS_ROW_PAD_Y: f32 = 8.0;
const SETTINGS_RADIUS_CONTROL: f32 = 6.0;
const SETTINGS_RADIUS_CARD: f32 = 10.0;
const SETTINGS_RADIUS_FULL: f32 = 999.0;
const SETTINGS_ROW_STACK_WIDTH: f32 = 640.0;
const SETTINGS_BUTTON_WIDTH: f32 = 82.0;
const SETTINGS_TOAST_MS: u64 = 2_400;
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
        let mut save_toast = None;
        if let Some((text, shown_at)) = &self.save_toast {
            if shown_at.elapsed() > Duration::from_millis(SETTINGS_TOAST_MS) {
                self.save_toast = None;
            } else {
                save_toast = Some(text.clone());
            }
        }
        let save_toast = save_toast.as_deref();
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
                    .inner_margin(egui::Margin::symmetric(22.0, 10.0)),
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
                    } else if let Some(message) = save_toast {
                        ui.add(
                            egui::Label::new(
                                RichText::new(message).small().color(palette.success),
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
                        settings_action_buttons(ui, self, palette, is_dirty, save_toast);
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
                            } else if let Some(message) = save_toast {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(message).small().color(palette.success),
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
                            if conflicts.is_empty() && save_toast.is_none() && self.status.is_empty() {
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
                            settings_action_buttons(ui, self, palette, is_dirty, save_toast);
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
                                            .size(11.0)
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
                                RichText::new("隐私说明：配置与日志仅保存在本机。")
                                    .strong()
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
                border_subtle: Color32::from_rgb(48, 48, 48),
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
                border_subtle: Color32::from_rgb(229, 235, 232),
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
        .rounding(SETTINGS_RADIUS_CONTROL)
        .min_size(egui::vec2(104.0, 34.0))
}

fn outline_icon_button<'a>(icon: &str, label: &'a str, palette: FluentPalette) -> egui::Button<'a> {
    egui::Button::new(RichText::new(format!("{icon}  {label}")).color(palette.text))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, palette.border))
        .rounding(SETTINGS_RADIUS_CONTROL)
        .min_size(egui::vec2(126.0, 34.0))
}

fn settings_action_buttons(
    ui: &mut egui::Ui,
    app: &mut SettingsApp,
    palette: FluentPalette,
    is_dirty: bool,
    save_toast: Option<&str>,
) {
    ui.horizontal(|ui| {
        if ui.add(fluent_primary_button(SAVE_CN, palette)).clicked() {
            let _ = app.save();
        }
        if is_dirty {
            status_dot(ui, palette.warning);
            ui.label(
                RichText::new("有未保存更改")
                    .size(SETTINGS_FONT_SMALL)
                    .color(palette.warning),
            );
        }
    });
    if ui
        .add(outline_icon_button("📁", OPEN_CFG_CN, palette))
        .clicked()
    {
        app.open_config_dir();
    }
    ui.menu_button("更多", |ui| {
        ui.set_min_width(150.0);
        if ui.button(RESET_CN).clicked() {
            app.reset_defaults();
            ui.close_menu();
        }
    });
}

fn fluent_nav_header(ui: &mut egui::Ui) {
    let palette = fluent_palette(ui);
    egui::Frame::none()
        .fill(palette.surface_alt)
        .stroke(Stroke::new(1.0, palette.border_subtle))
        .rounding(SETTINGS_RADIUS_CARD)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (mark, _) =
                    ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(mark.center(), 17.0, palette.accent);
                ui.painter().text(
                    mark.center(),
                    egui::Align2::CENTER_CENTER,
                    "开",
                    FontId::proportional(18.0),
                    palette.accent_text,
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(TITLE_CN)
                            .strong()
                            .size(SETTINGS_FONT_BRAND_TITLE)
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new("本地、干净、可控")
                            .small()
                            .color(palette.muted),
                    );
                    ui.label(
                        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(palette.muted),
                    );
                });
            });
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
    let radius = egui::Rounding::same(SETTINGS_RADIUS_CONTROL);
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
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), egui::Sense::click());
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
            egui::Rounding::same(SETTINGS_RADIUS_CONTROL),
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
        SettingsSection::Clipboard => None,
        // "词库与个性化"本身较长；在窄侧栏同一行追加数量会挤压标题。
        // 词库统计保留在页面内，导航只显示不会破坏布局的状态摘要。
        SettingsSection::Lexicon => None,
        SettingsSection::Compatibility => {
            let count = model.compat_rules.len();
            (count > 0).then(|| format!("{count} 规则"))
        }
        SettingsSection::Privacy => model.privacy_enabled.then(|| "隐私模式".to_string()),
        _ => None,
    }
}

fn section_panel(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let palette = fluent_palette(ui);
    ui.add_space(4.0);
    let available_width = ui.available_width();
    egui::Frame::none()
        .fill(palette.surface)
        .stroke(Stroke::new(0.5, palette.border_subtle))
        .rounding(SETTINGS_RADIUS_CARD)
        .inner_margin(egui::Margin::symmetric(16.0, 14.0))
        .show(ui, |ui| {
            ui.set_width((available_width - 36.0).max(160.0));
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
    ui.add_space(16.0);
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

fn tool_page_intro(ui: &mut egui::Ui, symbol: &str, title: &str, hint: &str) {
    let palette = fluent_palette(ui);
    egui::Frame::none()
        .fill(palette.surface_alt)
        .rounding(SETTINGS_PANEL_RADIUS)
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                tool_symbol(ui, symbol, palette.nav_selected);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(title)
                            .strong()
                            .size(SETTINGS_FONT_SECTION_TITLE)
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(hint)
                            .size(SETTINGS_FONT_SMALL)
                            .color(palette.muted),
                    );
                });
            });
        });
    ui.add_space(10.0);
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
    ui.painter().rect(
        rect,
        SETTINGS_RADIUS_CONTROL,
        fill,
        Stroke::new(1.0, palette.border_subtle),
    );
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
    let show_description = ui.available_width() >= SETTINGS_ROW_STACK_WIDTH;
    let row = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(0.0, SETTINGS_ROW_PAD_Y))
        .show(ui, |ui| {
            let total_width = ui.available_width();
            if total_width < SETTINGS_ROW_STACK_WIDTH {
                ui.vertical(|ui| {
                    ui.set_width(total_width);
                    ui.horizontal(|ui| {
                        tool_symbol(ui, symbol, palette.surface_alt);
                        ui.vertical(|ui| {
                            ui.set_width((total_width - 42.0).max(160.0));
                            let title_response = ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            if show_description {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(description)
                                            .size(SETTINGS_FONT_SMALL)
                                            .color(palette.muted),
                                    )
                                    .wrap(),
                                );
                            } else {
                                if !description.is_empty() {
                                    title_response.on_hover_text(description);
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(total_width, SETTINGS_CONTROL_HEIGHT),
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
                    ui.set_min_height(SETTINGS_ROW_HEIGHT);
                    tool_symbol(ui, symbol, palette.surface_alt);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, SETTINGS_ROW_HEIGHT),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            let title_response = ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            if show_description {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(description)
                                            .size(SETTINGS_FONT_SMALL)
                                            .color(palette.muted),
                                    )
                                    .wrap(),
                                );
                            } else {
                                if !description.is_empty() {
                                    title_response.on_hover_text(description);
                                }
                            }
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(control_width, SETTINGS_ROW_HEIGHT),
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
        Stroke::new(0.6, palette.border_subtle),
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
        .rounding(SETTINGS_RADIUS_FULL)
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
        .rounding(SETTINGS_RADIUS_FULL)
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
        .rounding(SETTINGS_RADIUS_CONTROL)
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
            .rounding(SETTINGS_RADIUS_CONTROL)
            .min_size(egui::vec2(SETTINGS_BUTTON_WIDTH, 34.0)),
    )
}

fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let palette = fluent_palette(ui);
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(palette.danger))
            .fill(palette.danger_bg)
            .stroke(Stroke::new(1.0, palette.danger))
            .rounding(SETTINGS_RADIUS_CONTROL)
            .min_size(egui::vec2(SETTINGS_BUTTON_WIDTH, 34.0)),
    )
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum RestartRequirement {
    #[default]
    None,
    FocusChange,
    ApplicationRestart,
}

#[derive(Clone, Copy, Debug)]
struct SettingSpec<'a> {
    title: &'a str,
    description: &'a str,
    visible: bool,
    enabled: bool,
    restart_requirement: RestartRequirement,
}

impl<'a> SettingSpec<'a> {
    fn new(title: &'a str, description: &'a str) -> Self {
        Self {
            title,
            description,
            visible: true,
            enabled: true,
            restart_requirement: RestartRequirement::None,
        }
    }
}

fn setting_toggle(ui: &mut egui::Ui, title: &str, description: &str, value: &mut bool) {
    setting_spec_row(ui, SettingSpec::new(title, description), |ui| {
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
        ui.add_sized([240.0, 24.0], Slider::new(value, range).show_value(true));
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
        ui.add_sized([240.0, 24.0], Slider::new(value, range).show_value(true));
    });
}

fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    setting_spec_row(ui, SettingSpec::new(title, description), add_control);
}

fn setting_spec_row(
    ui: &mut egui::Ui,
    spec: SettingSpec<'_>,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    if !spec.visible {
        return;
    }
    ui.add_enabled_ui(spec.enabled, |ui| {
        setting_spec_row_enabled(ui, spec, add_control);
    });
}

fn setting_spec_row_enabled(
    ui: &mut egui::Ui,
    spec: SettingSpec<'_>,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let title = spec.title;
    let description = spec.description;
    let palette = fluent_palette(ui);
    let show_description = ui.available_width() >= SETTINGS_ROW_STACK_WIDTH;
    let row = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(0.0, SETTINGS_ROW_PAD_Y - 1.0))
        .show(ui, |ui| {
            let total_width = ui.available_width();
            if total_width < SETTINGS_ROW_STACK_WIDTH {
                ui.vertical(|ui| {
                    ui.set_width(total_width);
                    let title_response = ui.label(
                        RichText::new(title)
                            .strong()
                            .size(SETTINGS_FONT_SETTING_TITLE)
                            .color(palette.text),
                    );
                    if !description.is_empty() && show_description {
                        ui.add(
                            egui::Label::new(
                                RichText::new(description)
                                    .size(SETTINGS_FONT_SMALL)
                                    .color(palette.muted),
                            )
                            .wrap(),
                        );
                    } else if !description.is_empty() {
                        title_response.on_hover_text(description);
                    }
                    render_restart_requirement(ui, spec.restart_requirement, palette);
                    ui.add_space(6.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(total_width, SETTINGS_ROW_HEIGHT),
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
                    ui.set_min_height(SETTINGS_ROW_HEIGHT);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, SETTINGS_ROW_HEIGHT),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            let title_response = ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(SETTINGS_FONT_SETTING_TITLE)
                                    .color(palette.text),
                            );
                            render_restart_requirement(ui, spec.restart_requirement, palette);
                            if !description.is_empty() && show_description {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(description)
                                            .size(SETTINGS_FONT_SMALL)
                                            .color(palette.muted),
                                    )
                                    .wrap(),
                                );
                            } else if !description.is_empty() {
                                title_response.on_hover_text(description);
                            }
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(control_width, SETTINGS_ROW_HEIGHT),
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
        Stroke::new(0.6, palette.border_subtle),
    );
}

fn render_restart_requirement(
    ui: &mut egui::Ui,
    requirement: RestartRequirement,
    palette: FluentPalette,
) {
    let label = match requirement {
        RestartRequirement::None => return,
        RestartRequirement::FocusChange => "切换窗口后生效",
        RestartRequirement::ApplicationRestart => "重启应用后生效",
    };
    ui.label(
        RichText::new(label)
            .size(SETTINGS_FONT_SMALL)
            .color(palette.muted),
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
            .rounding(SETTINGS_RADIUS_CARD)
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
    ui.horizontal_wrapped(|ui| {
        if outline_button(ui, "导入").clicked() {
            app.import_user_dict();
        }
        if outline_button(ui, "覆盖导入").clicked() {
            app.replace_user_dict();
        }
        if outline_button(ui, "导出完整备份").clicked() {
            app.export_user_dict();
        }
        if outline_button(ui, "导出便携词表").clicked() {
            app.export_user_dict_tsv();
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
        .rounding(SETTINGS_RADIUS_CARD)
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

#[path = "ui_diagnostics.rs"]
mod diagnostics;
use diagnostics::*;
pub(super) use diagnostics::{export_diagnostic_package_to, DiagnosticsSnapshot};

#[path = "ui_screenshot.rs"]
mod screenshot;
use screenshot::*;
#[path = "ui_ocr.rs"]
mod ocr;
use ocr::*;
#[path = "ui_translation.rs"]
mod translation;
use translation::*;

#[path = "ui_appearance.rs"]
mod appearance;
use appearance::*;
#[path = "ui_hotkeys_clipboard.rs"]
mod hotkeys_clipboard;
use hotkeys_clipboard::*;
#[path = "ui_privacy.rs"]
mod privacy;
use privacy::*;
pub(super) use privacy::{diagnostic_log_paths, privacy_statement_text};
