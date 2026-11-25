use super::*;
use pinyin_ime::ui_theme::UiDesignStyle;

#[path = "ui_controls.rs"]
mod controls;
pub(super) use controls::SettingsComboBoxExt;
use controls::{button, text_field, ButtonKind};

#[path = "ui_layout.rs"]
mod layout;
use layout::{bounded_control_width, responsive_settings_columns, responsive_settings_row};

#[path = "ui_lexicon_list.rs"]
mod lexicon_list;
use lexicon_list::{
    lexicon_coverage, lexicon_display_group, lexicon_group_hint, lexicon_list_row,
    lexicon_sort_key, set_lexicon_group_enabled,
};

#[cfg(test)]
#[path = "ui_layout_tests.rs"]
mod layout_tests;

const SETTINGS_FONT_HEADING: f32 = 20.0;
const SETTINGS_FONT_PAGE_TITLE: f32 = 20.0;
const SETTINGS_FONT_BRAND_TITLE: f32 = 15.0;
const SETTINGS_FONT_SECTION_TITLE: f32 = 15.0;
const SETTINGS_FONT_SETTING_TITLE: f32 = 14.0;
const SETTINGS_FONT_NAV_TITLE: f32 = 14.0;
const SETTINGS_FONT_BODY: f32 = 14.0;
const SETTINGS_MIN_HINT_FONT: f32 = 10.0;
const SETTINGS_FONT_SMALL: f32 = 13.0;
const SETTINGS_FONT_MONOSPACE: f32 = 13.0;
const SETTINGS_FONT_LOG: f32 = 12.0;
const SETTINGS_CONTROL_WIDTH: f32 = 200.0;
const SETTINGS_ROW_HEIGHT: f32 = 28.0;
const SETTINGS_ROW_PAD_Y: f32 = 0.0;
const SETTINGS_RADIUS_FULL: f32 = pinyin_ime::ui_theme::FLOATING_RADIUS;
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
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        pinyin_ime::ui_theme::clear_color_for_visuals(visuals)
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.apply_pending_appearance_changes(ctx);
        if self.pending_style_settle_frames > 0 {
            self.pending_style_settle_frames -= 1;
            ctx.request_repaint();
        }
        if self.preview_font_choice != self.model.candidate_font_file {
            self.preview_font_choice = self.model.candidate_font_file.clone();
            let _ = fonts::install_cjk_fonts_with_candidate_preview(
                ctx,
                Some(&self.preview_font_choice),
            );
        }
        if let Some(rx) = &self.translation_check_rx {
            match rx.try_recv() {
                Ok(status) => {
                    self.status = status;
                    self.translation_check_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "翻译检测异常结束".into();
                    self.translation_check_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
            }
        }
        if let Some(rx) = &self.diagnostics_rx {
            match rx.try_recv() {
                Ok(mut snapshot) => {
                    snapshot.refreshed_at = Instant::now();
                    self.diagnostics_cache = Some(snapshot);
                    self.diagnostics_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.diagnostics_rx = None,
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
            }
        }
        if let Some(rx) = &self.performance_export_rx {
            match rx.try_recv() {
                Ok(status) => {
                    self.status = status;
                    self.performance_export_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "性能日志导出任务异常结束，请重试。".to_string();
                    self.performance_export_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
            }
        }
        self.poll_user_dict_task();
        if self.user_dict_task_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let current_style_key = (
            pinyin_ime::ui_theme::active_design_style(),
            ctx.style().visuals.dark_mode,
        );
        if self.applied_style_key != Some(current_style_key) {
            enforce_settings_min_font_size(ctx);
            let synced_palette = FluentPalette::from_visuals(&ctx.style().visuals);
            sync_window_dwm_chrome(frame, synced_palette);
            self.applied_style_key = Some(current_style_key);
        }
        let palette = FluentPalette::from_visuals(&ctx.style().visuals);

        // `SettingsModel` is already `PartialEq`; comparing it avoids cloning
        // and rendering the whole INI file on every egui repaint.  Rendering
        // is deliberately kept on the save path, where it belongs.
        let is_dirty = self.model != self.last_saved_model;
        if !self.confirm_close_with_unsaved_changes {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
                && is_dirty
            {
                let _ = self.save();
            }
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                self.focus_search_requested = true;
            }
            if let Some(section) = controls::navigation_shortcut(ctx) {
                self.reset_section_scroll |= self.active_section != section;
                self.active_section = section;
            }
        }
        let is_dirty = self.model != self.last_saved_model;
        let mut request_real_candidate_preview = false;
        if is_dirty {
            self.save_toast = None;
        }
        let save_toast = self.save_toast.as_ref().map(|(text, _)| text.clone());
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
                        if ui.add(button("保存并关闭", ButtonKind::Primary)).clicked()
                            && self.save().is_ok()
                        {
                            self.confirm_close_with_unsaved_changes = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if danger_button(ui, "放弃更改").clicked() {
                            self.last_saved_model = self.model.clone();
                            self.confirm_close_with_unsaved_changes = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if outline_button(ui, "继续编辑").clicked() {
                            self.confirm_close_with_unsaved_changes = false;
                        }
                    });
                });
        }
        game_test_wizard_ui(ctx, self);
        show_settings_footer(ctx, self, palette, is_dirty, save_toast);

        let compact_navigation = ctx.available_rect().width() < SETTINGS_MIN_WINDOW_SIZE[0];
        if compact_navigation {
            egui::TopBottomPanel::top("settings_compact_nav")
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::none()
                        .fill(palette.nav_bg)
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
                )
                .show(ctx, |ui| {
                    let panel_rect = ui.clip_rect();
                    let y = ui
                        .painter()
                        .round_to_pixel_center(panel_rect.bottom() - 0.5);
                    ui.painter().hline(
                        panel_rect.x_range(),
                        y,
                        Stroke::new(1.0_f32, palette.border_subtle),
                    );
                    let previous_section = self.active_section;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(TITLE_CN).strong().color(palette.text));
                        ComboBox::from_id_salt("settings_compact_section")
                            .selected_text(self.active_section.label())
                            .width((ui.available_width() - 300.0).clamp(90.0, 260.0))
                            .truncate()
                            .show_settings(ui, |ui| {
                                for section in SettingsSection::ALL {
                                    ui.selectable_value(
                                        &mut self.active_section,
                                        section,
                                        section.label(),
                                    );
                                }
                            });
                        settings_style_picker(ui, self, 148.0);
                        settings_theme_picker(ui, self, 132.0);
                    });
                    if self.active_section != previous_section {
                        self.reset_section_scroll = true;
                    }
                });
        } else {
            egui::SidePanel::left("settings_nav")
                .resizable(false)
                .exact_width(SETTINGS_NAV_WIDTH)
                .show_separator_line(false)
                .frame(
                    egui::Frame::none()
                        .fill(palette.nav_bg)
                        .inner_margin(egui::Margin::symmetric(10.0, 10.0)),
                )
                .show(ctx, |ui| {
                    let panel_rect = ui.clip_rect();
                    let x = ui.painter().round_to_pixel_center(panel_rect.right() - 0.5);
                    ui.painter().vline(
                        x,
                        panel_rect.y_range(),
                        Stroke::new(1.0_f32, palette.border_subtle),
                    );
                    fluent_nav_header(ui);
                    ui.add_space(8.0);

                    let bottom_reserved = SETTINGS_ROW_HEIGHT * 2.0 + 26.0;
                    let nav_height = (ui.available_height() - bottom_reserved).max(80.0);
                    egui::ScrollArea::vertical()
                        .max_height(nav_height)
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
                                ui.add_space(2.0);
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
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        let picker_width = ui.available_width();
                        settings_theme_picker(ui, self, picker_width);
                        ui.add_space(2.0);
                        settings_style_picker(ui, self, picker_width);
                        ui.separator();
                    });
                });
        }

        if self.active_section == SettingsSection::Diagnostics {
            let needs_refresh = self
                .diagnostics_cache
                .as_ref()
                .map(|snapshot| snapshot.refreshed_at.elapsed() >= Duration::from_secs(2))
                .unwrap_or(true);
            if needs_refresh && self.diagnostics_rx.is_none() {
                let foreground = self.foreground_process.clone();
                let (tx, rx) = mpsc::channel();
                self.diagnostics_rx = Some(rx);
                let repaint = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(build_diagnostics_snapshot(foreground));
                    repaint.request_repaint();
                });
            }
            // Diagnostics are intentionally sampled, not rebuilt for every
            // mouse move or scroll event.  This keeps the page responsive
            // while still showing fresh runtime data.
            ctx.request_repaint_after(Duration::from_secs(2));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(palette.app_bg))
            .show(ctx, |ui| {
                settings_content_ui(ui, self, &mut request_real_candidate_preview);
            });
        if request_real_candidate_preview && self.save().is_ok() {
            self.status = match launch_real_candidate_preview() {
                Ok(()) => "已保存设置并弹出真实候选窗；预览将在 10 秒后关闭。".to_string(),
                Err(error) => format!("无法启动真实候选窗预览：{error}"),
            };
        }
    }
}

use std::cell::RefCell;

fn settings_style_picker(ui: &mut egui::Ui, app: &mut SettingsApp, width: f32) {
    let previous = pinyin_ime::ui_theme::active_design_style();
    let mut selected = app.pending_design_style.unwrap_or(previous);
    ComboBox::from_id_salt("settings_design_style")
        .selected_text(format!("风格：{}", selected.short_label()))
        .width(width)
        .truncate()
        .show_settings(ui, |ui| {
            for candidate in UiDesignStyle::ALL {
                ui.selectable_value(&mut selected, candidate, candidate.label())
                    .on_hover_text(candidate.subtitle());
            }
        });
    if selected != previous {
        app.queue_design_style_change(selected, None);
        ui.ctx().request_repaint();
    }
}

fn settings_theme_picker(ui: &mut egui::Ui, app: &mut SettingsApp, width: f32) {
    let active_style = pinyin_ime::ui_theme::active_design_style();
    let is_forced_dark = active_style == UiDesignStyle::LinearDark;
    let previous_pref = ui.ctx().options(|options| options.theme_preference);
    let mut preference = app.pending_theme_preference.unwrap_or(previous_pref);
    ui.add_enabled_ui(!is_forced_dark, |ui| {
        let label = if is_forced_dark {
            "明暗：深色（固定）"
        } else {
            match preference {
                egui::ThemePreference::System => "明暗：跟随系统",
                egui::ThemePreference::Dark => "明暗：深色",
                egui::ThemePreference::Light => "明暗：浅色",
            }
        };
        ComboBox::from_id_salt("settings_theme")
            .selected_text(label)
            .width(width)
            .truncate()
            .show_settings(ui, |ui| {
                ui.selectable_value(&mut preference, egui::ThemePreference::System, "跟随系统");
                ui.selectable_value(&mut preference, egui::ThemePreference::Dark, "深色");
                ui.selectable_value(&mut preference, egui::ThemePreference::Light, "浅色");
            });
    });
    if !is_forced_dark && preference != previous_pref {
        app.queue_theme_preference_change(preference);
        ui.ctx().request_repaint();
    }
}

thread_local! {
    static ACTIVE_SEARCH_FOCUS: RefCell<Option<(String, Instant)>> = const { RefCell::new(None) };
}

fn settings_content_ui(
    ui: &mut egui::Ui,
    app: &mut SettingsApp,
    request_real_candidate_preview: &mut bool,
) {
    if app
        .search_focus_target
        .as_ref()
        .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(3))
    {
        app.search_focus_target = None;
    }
    ACTIVE_SEARCH_FOCUS.with(|slot| {
        *slot.borrow_mut() = app.search_focus_target.clone();
    });
    if app.search_focus_target.is_some() {
        ui.ctx().request_repaint_after(Duration::from_millis(80));
    }
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(
            if ui.available_width() < 480.0 {
                12.0
            } else {
                16.0
            },
            12.0,
        ))
        .show(ui, |ui| {
            let content_width = ui.available_width().min(SETTINGS_CONTENT_MAX_WIDTH);
            ui.set_width(content_width);
            if let Some(page) = active_tool_page(app) {
                app.tool_page = page;
            }
            if app.active_section == SettingsSection::Learning
                && app.lexicon_page == LexiconSettingsPage::Extensions
            {
                app.lexicon_page = LexiconSettingsPage::Learning;
            }
            section_header(ui, app);
            settings_search_ui(ui, app, true);
            ui.add_space(4.0);
            match app.active_section {
                SettingsSection::Appearance => appearance_page_tabs(ui, app),
                SettingsSection::Learning => lexicon_page_tabs(ui, app),
                SettingsSection::System => system_page_tabs(ui, app),
                _ => {}
            }
            ui.add_space(8.0);

            let reset_scroll = app.reset_section_scroll;
            app.reset_section_scroll = false;
            let mut scroll_area = egui::ScrollArea::vertical()
                .id_salt("settings_section_content")
                .auto_shrink([false, false]);
            if reset_scroll {
                scroll_area = scroll_area.vertical_scroll_offset(0.0);
            }
            scroll_area.show(ui, |ui| {
                ui.set_width(ui.available_width().min(content_width));
                ui.data_mut(|data| data.insert_temp(controls::module_counter(), 0_usize));
                match app.active_section {
                    SettingsSection::Input => input_page_ui(ui, app),
                    SettingsSection::Appearance => {
                        candidate_page_ui(ui, app, request_real_candidate_preview)
                    }
                    SettingsSection::Lexicon => {
                        section_panel(ui, "扩展词库", |ui| extension_lexicon_controls(ui, app));
                    }
                    SettingsSection::Learning => lexicon_page_ui(ui, app),
                    SettingsSection::Diagnostics => diagnostics_page_ui(ui, app),
                    SettingsSection::Hotkeys => hotkeys_ui(ui, &mut app.model),
                    SettingsSection::Tools
                    | SettingsSection::Screenshot
                    | SettingsSection::Ocr
                    | SettingsSection::Clipboard
                    | SettingsSection::Translation
                    | SettingsSection::Handwrite => tools_page_ui(ui, app),
                    SettingsSection::Compatibility => compatibility_ui(ui, app),
                    SettingsSection::System => system_page_ui(ui, app),
                }
            });
        });
}

pub(super) type FluentPalette = pinyin_ime::ui_theme::UiPalette;

pub(super) fn fluent_palette(ui: &egui::Ui) -> FluentPalette {
    FluentPalette::from_visuals(ui.visuals())
}

fn settings_action_buttons(ui: &mut egui::Ui, app: &mut SettingsApp, is_dirty: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.menu_button("更多", |ui| {
            ui.set_min_width(150.0);
            if ui.button(OPEN_CFG_CN).clicked() {
                app.open_config_dir();
                ui.close_menu();
            }
            if ui.button("恢复全部默认").clicked() {
                let confirmed = matches!(
                    rfd::MessageDialog::new()
                        .set_title("恢复全部默认设置")
                        .set_description("这会重置所有输入、外观、工具、兼容和隐私选项。用户词库和历史数据不会被删除。")
                        .set_level(rfd::MessageLevel::Warning)
                        .set_buttons(rfd::MessageButtons::OkCancel)
                        .show(),
                    rfd::MessageDialogResult::Ok
                );
                if confirmed { app.reset_defaults(); }
                ui.close_menu();
            }
        });
        if is_dirty {
            if outline_button(ui, "放弃修改").clicked() {
                app.model = app.last_saved_model.clone();
                app.status = "已撤销未保存更改。".to_string();
            }
        } else {
            ui.allocate_space(egui::vec2(SETTINGS_BUTTON_WIDTH, SETTINGS_ROW_HEIGHT));
        }
        if ui.add_enabled(is_dirty, button(SAVE_CN, ButtonKind::Primary)).clicked() {
            let _ = app.save();
        }
    });
}

fn footer_status(ui: &mut egui::Ui, text: &str, color: Color32, monospace: bool) {
    ui.set_min_height(38.0);
    let mut text_style = RichText::new(text).small().color(color);
    if monospace {
        text_style = text_style.monospace();
    }
    let mut job = egui::WidgetText::from(text_style).into_layout_job(
        ui.style(),
        egui::FontSelection::Default,
        egui::Align::Min,
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    ui.add(egui::Label::new(job).wrap()).on_hover_text(text);
}

fn show_settings_footer(
    ctx: &egui::Context,
    app: &mut SettingsApp,
    palette: FluentPalette,
    is_dirty: bool,
    save_toast: Option<&str>,
) -> egui::Rect {
    let bottom = ctx.available_rect().bottom();
    let rect = egui::TopBottomPanel::bottom("settings_actions")
        .resizable(false)
        .show_separator_line(false)
        .frame(controls::footer_outer_frame(palette))
        .show(ctx, |ui| {
            controls::show_footer_container(ui, palette, |ui| {
                settings_footer_ui(ui, app, palette, is_dirty, save_toast);
            });
        })
        .response
        .rect;
    if (rect.bottom() - bottom).abs() > 0.5 {
        // The panel starts from its cached height. Lay out again with the new height before painting.
        ctx.request_discard("Settings footer height changed");
    }
    rect
}

fn settings_footer_ui(
    ui: &mut egui::Ui,
    app: &mut SettingsApp,
    palette: FluentPalette,
    is_dirty: bool,
    save_toast: Option<&str>,
) {
    let conflicts = hotkey_conflicts(&app.model);
    let (status, color) = if !conflicts.is_empty() {
        (
            format!("快捷键冲突：{}", conflicts.join(" / ")),
            palette.warning,
        )
    } else if is_dirty {
        ("● 有未保存的修改".to_string(), palette.warning)
    } else if let Some(message) = save_toast {
        (message.to_string(), palette.success)
    } else if !app.status.is_empty() {
        (app.status.clone(), palette.muted)
    } else {
        (
            "设置保存在本机；多数设置保存后即时生效。".to_string(),
            palette.muted,
        )
    };
    let width = ui.available_width();
    let actions_width = 272.0_f32.min(width);
    let status_height = if palette.style == UiDesignStyle::TactileIndustrial {
        44.0
    } else {
        38.0
    };
    if width < actions_width + 160.0 {
        ui.allocate_ui_with_layout(
            egui::vec2(width, status_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width);
                controls::footer_readout(ui, palette, &status, color, is_dirty);
            },
        );
        ui.horizontal(|ui| {
            ui.add_space((width - actions_width).max(0.0));
            ui.allocate_ui_with_layout(
                egui::vec2(actions_width, SETTINGS_ROW_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(actions_width);
                    settings_action_buttons(ui, app, is_dirty);
                },
            );
        });
    } else {
        ui.horizontal_top(|ui| {
            let status_width = width - actions_width - ui.spacing().item_spacing.x;
            ui.allocate_ui_with_layout(
                egui::vec2(status_width, status_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(status_width);
                    controls::footer_readout(ui, palette, &status, color, is_dirty);
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(actions_width, status_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(actions_width);
                    settings_action_buttons(ui, app, is_dirty);
                },
            );
        });
    }
}

fn fluent_nav_header(ui: &mut egui::Ui) {
    let palette = fluent_palette(ui);
    ui.horizontal(|ui| {
        let (mark, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(mark, palette.control_radius, palette.accent);
        if palette.style == pinyin_ime::ui_theme::UiDesignStyle::TactileIndustrial {
            ui.painter().rect_stroke(
                mark,
                egui::Rounding::same(palette.control_radius),
                Stroke::new(1.2_f32, palette.plate_border),
            );
        }
        ui.painter().text(
            mark.center(),
            egui::Align2::CENTER_CENTER,
            "开",
            FontId::proportional(16.0),
            palette.accent_text,
        );
        ui.vertical(|ui| {
            ui.label(
                RichText::new("开心输入法")
                    .strong()
                    .size(SETTINGS_FONT_BRAND_TITLE),
            );
            let sub = if palette.style == pinyin_ime::ui_theme::UiDesignStyle::TactileIndustrial {
                format!("KX-05 · v{}", env!("CARGO_PKG_VERSION"))
            } else {
                format!("设置 · v{}", env!("CARGO_PKG_VERSION"))
            };
            ui.label(RichText::new(sub).small().color(palette.muted));
        });
    });
}

fn section_header(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    controls::header_frame(palette).show(ui, |ui| {
        ui.horizontal(|ui| {
            controls::page_title(ui, app.active_section, palette);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if outline_button(ui, "恢复本页推荐").clicked() {
                    reset_active_page_defaults(app);
                }
            });
        });
    });
}

fn reset_active_page_defaults(app: &mut SettingsApp) {
    let defaults = SettingsModel::default();
    macro_rules! copy_fields {
        ($($field:ident),+ $(,)?) => {{
            $(app.model.$field = defaults.$field.clone();)+
        }};
    }
    match app.active_section {
        SettingsSection::Input => copy_fields!(
            default_ascii,
            global_ascii,
            default_full_shape,
            default_chinese_punct,
            curly_punct,
            auto_pair_punct,
            number_fullwidth,
            symbol_fullwidth,
            chinese_halfwidth,
            shift_symbol_temporary_ascii,
            date_auto_format,
            english_word_input,
            traditional_output,
            default_fuzzy_pinyin,
            fuzzy_zh_z,
            fuzzy_ch_c,
            fuzzy_sh_s,
            fuzzy_n_l,
            fuzzy_f_h,
            fuzzy_an_ang,
            fuzzy_en_eng,
            fuzzy_in_ing,
            default_double_pinyin,
            double_pinyin_schema,
            jianpin,
            mixed_pinyin,
            mixed_pinyin_aggressive,
            correction_enabled,
            retry_on_failure,
            u_mode,
        ),
        SettingsSection::Appearance => apply_candidate_recommended_defaults(&mut app.model),
        SettingsSection::Lexicon => {
            for (tag, enabled) in &mut app.model.lexicon_tags {
                *enabled = pinyin_ime::lexicon_prefs::default_optional_lexicon_tag_enabled(tag);
            }
        }
        SettingsSection::Learning => copy_fields!(learning_sensitivity, user_hotword_boost),
        SettingsSection::Diagnostics => copy_fields!(log_level),
        SettingsSection::Hotkeys => copy_fields!(
            cn_en_hotkey,
            full_shape_hotkey,
            punct_hotkey,
            fuzzy_hotkey,
            double_pinyin_hotkey,
            shift_tap_hotkey,
            candidate_number_select,
            page_minus_equal,
            page_comma_period,
            page_pgup_pgdn,
            screenshot_hotkey,
            clipboard_hotkey,
            settings_hotkey,
            handwrite_hotkey,
            ocr_hotkey,
            ocr_translate_hotkey,
            translate_hotkey,
            traditional_hotkey,
            game_mode_hotkey,
            temporary_ascii_hotkey,
        ),
        SettingsSection::Tools
        | SettingsSection::Screenshot
        | SettingsSection::Ocr
        | SettingsSection::Clipboard
        | SettingsSection::Translation
        | SettingsSection::Handwrite => match active_tool_page(app).unwrap() {
            ToolSettingsPage::Clipboard => copy_fields!(
                clipboard_background_enabled,
                clipboard_max_history_items,
                clipboard_max_pinned_items,
                clipboard_max_text_utf16_units,
                clipboard_max_age_days,
                clipboard_candidate_preview_enabled,
                clipboard_record_source_app,
                clipboard_pinned_respects_max_age,
            ),
            ToolSettingsPage::Screenshot => copy_fields!(
                screenshot_auto_save,
                screenshot_copy_after_capture,
                screenshot_ocr_after_capture,
                screenshot_translate_after_capture,
                screenshot_save_dir,
                screenshot_silent_copy_enabled,
                screenshot_silent_copy_dir,
                screenshot_name_pattern,
                screenshot_date_subdirs,
                screenshot_conflict_strategy,
                screenshot_format,
                screenshot_mode,
                screenshot_confirm_on_release,
                screenshot_show_instructions,
                screenshot_show_quick_actions,
                screenshot_show_result_window,
            ),
            ToolSettingsPage::Ocr => copy_fields!(
                ocr_result_action,
                ocr_keep_alive,
                ocr_profile,
                ocr_translate_keep_window,
                ocr_screenshot_auto_save,
                ocr_screenshot_save_dir,
                ocr_screenshot_name_pattern,
            ),
            ToolSettingsPage::Handwrite => {}
            ToolSettingsPage::Commands => copy_fields!(v_assist, symbol_toolbox, emoji_input),
            ToolSettingsPage::Translation => {
                copy_fields!(
                    wintranslator_path,
                    translate_result_action,
                    translate_target_language
                )
            }
        },
        SettingsSection::Compatibility => copy_fields!(
            fullscreen_detection,
            fullscreen_policy,
            commit_transport,
            builtin_game_list,
            auto_suggest_app_options,
        ),
        SettingsSection::System => match app.system_page {
            SystemSettingsPage::General => copy_fields!(
                show_status_notifications,
                show_notifications,
                show_notifications_time
            ),
            SystemSettingsPage::About => {}
            SystemSettingsPage::Privacy => copy_fields!(
                privacy_enabled,
                privacy_never_learn_processes,
                privacy_never_clipboard_processes,
                privacy_never_candidate_processes,
            ),
            SystemSettingsPage::Diagnostics => {
                copy_fields!(log_level)
            }
            SystemSettingsPage::Advanced => {
                app.reset_rank_defaults();
                app.reset_engine_tuning_defaults();
            }
        },
    }
    app.status = "当前页面已恢复推荐设置；保存后生效。".to_string();
}

pub(super) fn enforce_settings_min_font_size(ctx: &egui::Context) {
    let active_style = pinyin_ime::ui_theme::active_design_style();
    for (theme, dark_mode) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        ctx.style_mut_of(theme, |style| {
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
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.button_padding = egui::vec2(7.0, 3.0);
            style.spacing.icon_width = 16.0;
            style.spacing.icon_width_inner = 10.0;
            style.interaction.tooltip_delay = 0.35;
            style.spacing.tooltip_width = 420.0;
            style.spacing.interact_size.y = SETTINGS_ROW_HEIGHT;

            let palette = FluentPalette::for_style(active_style, dark_mode);
            pinyin_ime::ui_theme::apply_palette_to_visuals(&mut style.visuals, palette);
        });
    }
}

fn nav_item(
    ui: &mut egui::Ui,
    current: &mut SettingsSection,
    section: SettingsSection,
    model: &SettingsModel,
) {
    let hint = nav_status_summary(section, model)
        .map(|summary| format!("{} · {summary}", section.hint()))
        .unwrap_or_else(|| section.hint().to_owned());
    if controls::navigation(ui, section, *current == section, hint).clicked() {
        *current = section;
    }
}

fn nav_status_summary(section: SettingsSection, model: &SettingsModel) -> Option<String> {
    match section {
        SettingsSection::Hotkeys => {
            let conflicts = hotkey_conflicts(model).len();
            (conflicts > 0).then(|| format!("{conflicts} 冲突"))
        }
        SettingsSection::Clipboard => model
            .clipboard_background_enabled
            .then(|| "剪贴板开".to_string()),
        // "词库与个性化"本身较长；在窄侧栏同一行追加数量会挤压标题。
        // 词库统计保留在页面内，导航只显示不会破坏布局的状态摘要。
        SettingsSection::Lexicon => None,
        SettingsSection::Compatibility => {
            let count = model.compat_rules.len();
            (count > 0).then(|| format!("{count} 规则"))
        }
        SettingsSection::System => model.privacy_enabled.then(|| "隐私模式".to_string()),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct SettingsSearchEntry {
    title: &'static str,
    hint: &'static str,
    keywords: &'static str,
    section: SettingsSection,
    tool_page: Option<ToolSettingsPage>,
    system_page: Option<SystemSettingsPage>,
    appearance_page: Option<AppearanceSettingsPage>,
    lexicon_page: Option<LexiconSettingsPage>,
}

const SETTINGS_SEARCH_ENTRIES: &[SettingsSearchEntry] = &[
    SettingsSearchEntry {
        title: "手写查字",
        hint: "独立手写工具",
        keywords: "handwrite 手写",
        section: SettingsSection::Handwrite,
        tool_page: Some(ToolSettingsPage::Handwrite),
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "VV 命令",
        hint: "日期、符号和 Emoji",
        keywords: "vv 日期 时间 符号 emoji",
        section: SettingsSection::Tools,
        tool_page: Some(ToolSettingsPage::Commands),
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "通知",
        hint: "状态提示与显示时长",
        keywords: "notification 通知",
        section: SettingsSection::System,
        tool_page: None,
        system_page: Some(SystemSettingsPage::General),
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "系统输入法",
        hint: "Windows 输入法与键盘管理",
        keywords: "微软拼音 键盘 系统输入法",
        section: SettingsSection::System,
        tool_page: None,
        system_page: Some(SystemSettingsPage::General),
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "关于",
        hint: "版本信息",
        keywords: "version 版本 关于",
        section: SettingsSection::System,
        tool_page: None,
        system_page: Some(SystemSettingsPage::About),
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "模糊音",
        hint: "拼音能力与模糊音细项",
        keywords: "z zh c ch s sh n l",
        section: SettingsSection::Input,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "双拼方案",
        hint: "双拼开关与方案",
        keywords: "double pinyin",
        section: SettingsSection::Input,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "简拼与混拼",
        hint: "简拼、混拼和输入纠错",
        keywords: "jianpin mixed correction",
        section: SettingsSection::Input,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "中文标点",
        hint: "半角字符、全角、弯引号和符号补全",
        keywords: "punctuation 全角 半角 halfwidth",
        section: SettingsSection::Input,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "英文候选",
        hint: "英文单词候选与输入工具",
        keywords: "english",
        section: SettingsSection::Input,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "候选数量",
        hint: "每页数量和横竖布局",
        keywords: "page size 分页",
        section: SettingsSection::Appearance,
        tool_page: None,
        system_page: None,
        appearance_page: Some(AppearanceSettingsPage::Layout),
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "候选字体",
        hint: "字体、字号和字重",
        keywords: "font",
        section: SettingsSection::Appearance,
        tool_page: None,
        system_page: None,
        appearance_page: Some(AppearanceSettingsPage::Theme),
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "界面视觉风格",
        hint: "5 款设置页与工具箱设计风格（默认风格 5 工业硬件风）",
        keywords: "style design 风格 工业 云母 bento linear 水墨",
        section: SettingsSection::Appearance,
        tool_page: None,
        system_page: None,
        appearance_page: Some(AppearanceSettingsPage::Theme),
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "皮肤主题",
        hint: "主题、透明度和材质",
        keywords: "theme opacity",
        section: SettingsSection::Appearance,
        tool_page: None,
        system_page: None,
        appearance_page: Some(AppearanceSettingsPage::Theme),
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "用户词库",
        hint: "学习偏好、自定义词条和屏蔽词",
        keywords: "dictionary learning",
        section: SettingsSection::Learning,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: Some(LexiconSettingsPage::User),
    },
    SettingsSearchEntry {
        title: "快捷键冲突",
        hint: "全局快捷键和翻页按键",
        keywords: "hotkey",
        section: SettingsSection::Hotkeys,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "剪贴板历史",
        hint: "采集、上限和敏感应用",
        keywords: "clipboard",
        section: SettingsSection::Clipboard,
        tool_page: Some(ToolSettingsPage::Clipboard),
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "截图目录",
        hint: "截图保存、命名和格式",
        keywords: "screenshot save path",
        section: SettingsSection::Screenshot,
        tool_page: Some(ToolSettingsPage::Screenshot),
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "OCR 模型",
        hint: "本地识别档位和结果操作",
        keywords: "识别",
        section: SettingsSection::Ocr,
        tool_page: Some(ToolSettingsPage::Ocr),
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "HY-MT2 / WinTranslator",
        hint: "翻译程序和译文处理",
        keywords: "翻译 translate",
        section: SettingsSection::Translation,
        tool_page: Some(ToolSettingsPage::Translation),
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "游戏兼容",
        hint: "应用规则、Overlay 和上屏方式",
        keywords: "fullscreen overlay",
        section: SettingsSection::Compatibility,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "隐私模式",
        hint: "敏感应用和本地数据",
        keywords: "privacy",
        section: SettingsSection::System,
        tool_page: None,
        system_page: Some(SystemSettingsPage::Privacy),
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "诊断日志",
        hint: "运行状态、延迟和日志",
        keywords: "diagnostics log 性能 日志 延迟",
        section: SettingsSection::Diagnostics,
        tool_page: None,
        system_page: Some(SystemSettingsPage::Diagnostics),
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "缓存容量",
        hint: "候选缓存和性能预算",
        keywords: "cache performance",
        section: SettingsSection::System,
        tool_page: None,
        system_page: Some(SystemSettingsPage::Advanced),
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "候选排序权重",
        hint: "单字、词组和语言模型权重",
        keywords: "ranking weight",
        section: SettingsSection::System,
        tool_page: None,
        system_page: Some(SystemSettingsPage::Advanced),
        appearance_page: None,
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "候选来源与调试标记",
        hint: "读音、分数、来源与纠错标记",
        keywords: "reading score source debug",
        section: SettingsSection::Appearance,
        tool_page: None,
        system_page: None,
        appearance_page: Some(AppearanceSettingsPage::Advanced),
        lexicon_page: None,
    },
    SettingsSearchEntry {
        title: "学习策略",
        hint: "学习灵敏度和热词提前力度",
        keywords: "learning 学习 热词",
        section: SettingsSection::Learning,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: Some(LexiconSettingsPage::Learning),
    },
    SettingsSearchEntry {
        title: "扩展词库",
        hint: "搜索、启用、导入和统计",
        keywords: "lexicon 专业词库 导入 词库名称",
        section: SettingsSection::Lexicon,
        tool_page: None,
        system_page: None,
        appearance_page: None,
        lexicon_page: Some(LexiconSettingsPage::Extensions),
    },
];

fn settings_search_ui(ui: &mut egui::Ui, app: &mut SettingsApp, compact: bool) {
    let palette = fluent_palette(ui);
    ui.horizontal(|ui| {
        let clear_space = if app.settings_search.is_empty() {
            0.0
        } else if compact {
            48.0
        } else {
            44.0
        };
        let width = (ui.available_width() - clear_space).max(1.0);
        let search_response = ui.add_sized(
            [width, SETTINGS_ROW_HEIGHT],
            text_field(
                TextEdit::singleline(&mut app.settings_search).hint_text("搜索设置 (Ctrl+F)"),
            ),
        );
        if app.focus_search_requested {
            search_response.request_focus();
            app.focus_search_requested = false;
        }
        if !app.settings_search.is_empty()
            && ui
                .add(compact_button("×", false))
                .on_hover_text("清除搜索")
                .clicked()
        {
            app.settings_search.clear();
        }
    });
    let query = app.settings_search.trim().to_lowercase();
    if query.is_empty() {
        return;
    }
    ui.add_space(4.0);
    let mut matches = 0usize;
    for entry in SETTINGS_SEARCH_ENTRIES {
        let haystack = format!("{} {} {}", entry.title, entry.hint, entry.keywords).to_lowercase();
        if !haystack.contains(&query) {
            continue;
        }
        matches += 1;
        if matches > 6 {
            break;
        }
        let response = ui.selectable_label(false, format!("{}  ·  {}", entry.title, entry.hint));
        if response.clicked() {
            navigate_to_search_entry(app, entry);
        }
    }
    if matches == 0 {
        ui.label(RichText::new("没有匹配的设置").small().color(palette.muted));
    }
}

fn navigate_to_search_entry(app: &mut SettingsApp, entry: &SettingsSearchEntry) {
    app.active_section = entry.section;
    if let Some(page) = entry.tool_page {
        app.tool_page = page;
    }
    if let Some(page) = entry.system_page {
        app.system_page = page;
    }
    if let Some(page) = entry.appearance_page {
        app.appearance_page = page;
    }
    if let Some(page) = entry.lexicon_page {
        app.lexicon_page = page;
    }
    app.status = format!("已定位到：{}", entry.title);
    app.search_focus_target = Some((entry.title.to_string(), Instant::now()));
    app.settings_search.clear();
    app.reset_section_scroll = true;
}

fn section_panel(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    controls::panel(ui, title, add_contents);
}

fn quiet_section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    section_panel(ui, title, add_contents);
}

fn paint_settings_icon(ui: &egui::Ui, center: egui::Pos2, icon: SettingsIcon, color: Color32) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.6_f32, color);
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
        SettingsIcon::Input => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(19.0, 13.0)));
            for x in [-5.0, -1.0, 3.0] {
                painter.circle_filled(center + egui::vec2(x, -2.5), 1.0, color);
            }
            line(
                center + egui::vec2(-5.0, 3.0),
                center + egui::vec2(5.0, 3.0),
            );
        }
        SettingsIcon::Appearance => {
            painter.circle_stroke(center, 8.0, stroke);
            painter.circle_filled(center + egui::vec2(-4.0, -2.0), 1.5, color);
            painter.circle_filled(center + egui::vec2(0.0, -5.0), 1.5, color);
            painter.circle_filled(center + egui::vec2(4.0, -2.0), 1.5, color);
            painter.circle_stroke(center + egui::vec2(3.0, 4.0), 2.2, stroke);
        }
        SettingsIcon::Lexicon => {
            line(
                center + egui::vec2(0.0, -6.0),
                center + egui::vec2(0.0, 8.0),
            );
            line(
                center + egui::vec2(-8.0, -6.0),
                center + egui::vec2(-8.0, 6.0),
            );
            line(
                center + egui::vec2(8.0, -6.0),
                center + egui::vec2(8.0, 6.0),
            );
            line(
                center + egui::vec2(-8.0, -6.0),
                center + egui::vec2(-2.0, -5.0),
            );
            line(
                center + egui::vec2(8.0, -6.0),
                center + egui::vec2(2.0, -5.0),
            );
            line(
                center + egui::vec2(-8.0, 6.0),
                center + egui::vec2(0.0, 8.0),
            );
            line(center + egui::vec2(8.0, 6.0), center + egui::vec2(0.0, 8.0));
        }
        SettingsIcon::Learning => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(16.0, 17.0)));
            line(
                center + egui::vec2(-5.0, -4.0),
                center + egui::vec2(2.0, -4.0),
            );
            line(
                center + egui::vec2(-5.0, 0.0),
                center + egui::vec2(1.0, 0.0),
            );
            line(
                center + egui::vec2(0.0, 5.0),
                center + egui::vec2(8.0, -3.0),
            );
            line(center + egui::vec2(0.0, 5.0), center + egui::vec2(3.0, 5.0));
        }
        SettingsIcon::Hotkeys => {
            line(
                center + egui::vec2(1.0, -9.0),
                center + egui::vec2(-5.0, 1.0),
            );
            line(
                center + egui::vec2(-5.0, 1.0),
                center + egui::vec2(0.0, 1.0),
            );
            line(
                center + egui::vec2(0.0, 1.0),
                center + egui::vec2(-1.0, 9.0),
            );
            line(
                center + egui::vec2(-1.0, 9.0),
                center + egui::vec2(6.0, -2.0),
            );
            line(
                center + egui::vec2(6.0, -2.0),
                center + egui::vec2(1.0, -2.0),
            );
            line(
                center + egui::vec2(1.0, -2.0),
                center + egui::vec2(1.0, -9.0),
            );
        }
        SettingsIcon::Screenshot => {
            for (a, b) in [
                ((-8.0, -3.0), (-8.0, -8.0)),
                ((-8.0, -8.0), (-3.0, -8.0)),
                ((8.0, -3.0), (8.0, -8.0)),
                ((8.0, -8.0), (3.0, -8.0)),
                ((-8.0, 3.0), (-8.0, 8.0)),
                ((-8.0, 8.0), (-3.0, 8.0)),
                ((8.0, 3.0), (8.0, 8.0)),
                ((8.0, 8.0), (3.0, 8.0)),
            ] {
                line(center + egui::vec2(a.0, a.1), center + egui::vec2(b.0, b.1));
            }
            painter.circle_stroke(center, 2.5, stroke);
        }
        SettingsIcon::Ocr => {
            rounded_rect(egui::Rect::from_center_size(center, egui::vec2(14.0, 18.0)));
            line(
                center + egui::vec2(-9.0, 2.0),
                center + egui::vec2(9.0, 2.0),
            );
            line(
                center + egui::vec2(-3.0, -4.0),
                center + egui::vec2(3.0, -4.0),
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
        SettingsIcon::Translation => {
            rounded_rect(egui::Rect::from_center_size(
                center + egui::vec2(-3.0, -2.0),
                egui::vec2(11.0, 12.0),
            ));
            rounded_rect(egui::Rect::from_center_size(
                center + egui::vec2(4.0, 3.0),
                egui::vec2(10.0, 11.0),
            ));
            painter.text(
                center + egui::vec2(-3.0, -2.0),
                egui::Align2::CENTER_CENTER,
                "文",
                FontId::proportional(8.0),
                color,
            );
            painter.text(
                center + egui::vec2(4.0, 3.0),
                egui::Align2::CENTER_CENTER,
                "A",
                FontId::proportional(8.0),
                color,
            );
        }
        SettingsIcon::Handwrite => {
            line(
                center + egui::vec2(-7.0, 7.0),
                center + egui::vec2(5.0, -7.0),
            );
            line(
                center + egui::vec2(-4.0, 8.0),
                center + egui::vec2(8.0, -5.0),
            );
            line(
                center + egui::vec2(5.0, -7.0),
                center + egui::vec2(8.0, -5.0),
            );
            line(
                center + egui::vec2(-7.0, 7.0),
                center + egui::vec2(-4.0, 8.0),
            );
            line(
                center + egui::vec2(-8.0, 9.0),
                center + egui::vec2(4.0, 9.0),
            );
        }
        SettingsIcon::Tools => {
            line(
                center + egui::vec2(-4.0, -8.0),
                center + egui::vec2(-8.0, 0.0),
            );
            line(
                center + egui::vec2(-8.0, 0.0),
                center + egui::vec2(-4.0, 8.0),
            );
            line(
                center + egui::vec2(4.0, -8.0),
                center + egui::vec2(8.0, 0.0),
            );
            line(center + egui::vec2(8.0, 0.0), center + egui::vec2(4.0, 8.0));
            painter.circle_filled(center, 1.5, color);
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
        SettingsIcon::Diagnostics => {
            line(
                center + egui::vec2(-9.0, 2.0),
                center + egui::vec2(-5.0, 2.0),
            );
            line(
                center + egui::vec2(-5.0, 2.0),
                center + egui::vec2(-2.0, -5.0),
            );
            line(
                center + egui::vec2(-2.0, -5.0),
                center + egui::vec2(1.0, 7.0),
            );
            line(center + egui::vec2(1.0, 7.0), center + egui::vec2(4.0, 0.0));
            line(center + egui::vec2(4.0, 0.0), center + egui::vec2(9.0, 0.0));
        }
        SettingsIcon::System => {
            painter.circle_stroke(center, 7.0, stroke);
            painter.circle_filled(center, 2.0, color);
            for angle in [
                0.0_f32,
                std::f32::consts::FRAC_PI_2,
                std::f32::consts::PI,
                3.0 * std::f32::consts::FRAC_PI_2,
            ] {
                let direction = egui::vec2(angle.cos(), angle.sin());
                line(center + direction * 7.0, center + direction * 9.0);
            }
        }
    }
}

fn tool_row(
    ui: &mut egui::Ui,
    _symbol: &str,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    setting_row(ui, title, description, add_control);
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
        StatusTone::Info => (palette.accent, palette.info_soft),
        StatusTone::Success => (palette.success, palette.success_soft),
        StatusTone::Warning => (palette.warning, palette.warning_soft),
        StatusTone::Danger => (palette.danger, palette.danger_soft),
    }
}

fn status_badge(ui: &mut egui::Ui, tone: StatusTone, text: &str) {
    let palette = fluent_palette(ui);
    let (color, fill) = status_tone_colors(palette, tone);
    let max_width = (ui.available_width() - 18.0).max(1.0);
    egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, color))
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
        .stroke(Stroke::new(1.0_f32, color))
        .rounding(palette.card_radius)
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

fn diagnostic_status_table(ui: &mut egui::Ui, items: &[(&str, &str, Color32)]) {
    let palette = fluent_palette(ui);
    let width = ui.available_width();
    for (index, (label, value, color)) in items.iter().enumerate() {
        egui::Frame::none()
            .fill(if index % 2 == 0 {
                palette.surface_alt
            } else {
                palette.surface
            })
            .inner_margin(egui::Margin::symmetric(6.0, 2.0))
            .show(ui, |ui| {
                ui.set_width((width - 12.0).max(1.0));
                responsive_settings_row(
                    ui,
                    (width - 190.0).max(1.0),
                    |ui| {
                        ui.label(RichText::new(*label).color(palette.muted));
                    },
                    |ui| {
                        status_dot(ui, *color);
                        ui.add(egui::Label::new(*value).wrap())
                            .on_hover_text(*value);
                    },
                );
            });
    }
}

fn outline_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(button(label, ButtonKind::Secondary))
}

pub(super) fn hotkey_modifier(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    value: &mut bool,
) -> bool {
    if ui
        .add_enabled(enabled, button(label, ButtonKind::Key(*value)))
        .clicked()
    {
        *value = !*value;
        true
    } else {
        false
    }
}

pub(super) fn compact_button(label: &str, selected: bool) -> impl egui::Widget + '_ {
    button(label, ButtonKind::Compact(selected))
}

fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(button(label, ButtonKind::Danger))
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
    control_width: f32,
}

impl<'a> SettingSpec<'a> {
    fn new(title: &'a str, description: &'a str) -> Self {
        Self {
            title,
            description,
            visible: true,
            enabled: true,
            restart_requirement: RestartRequirement::None,
            control_width: SETTINGS_CONTROL_WIDTH,
        }
    }
}

fn setting_toggle(ui: &mut egui::Ui, title: &str, description: &str, value: &mut bool) {
    setting_row(ui, title, description, |ui| {
        controls::switch(ui, title, value);
    });
}

fn setting_text(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    requirement: RestartRequirement,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), SETTINGS_ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(title)
                        .size(SETTINGS_FONT_SETTING_TITLE)
                        .color(fluent_palette(ui).text),
                )
                .wrap(),
            )
            .on_hover_text(setting_help(description, requirement));
        },
    );
}

fn setting_help(description: &str, requirement: RestartRequirement) -> String {
    let effect = match requirement {
        RestartRequirement::None => "",
        RestartRequirement::FocusChange => "切换窗口后生效。",
        RestartRequirement::ApplicationRestart => "重启应用后生效。",
    };
    if effect.is_empty() {
        description.to_owned()
    } else if description.is_empty() {
        effect.to_owned()
    } else {
        format!("{description}\n{effect}")
    }
}

// Scope responses include the complete row, including blank space and controls.
// Disabled settings retain their help, and child controls keep their own input.
fn setting_row_help(response: egui::Response, help: &str) {
    response.on_hover_text(help).on_disabled_hover_text(help);
}

pub(super) fn capsule_switch(ui: &mut egui::Ui, value: &mut bool) -> egui::Response {
    controls::switch(ui, "开关", value)
}

fn setting_slider_usize(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut usize,
    range: std::ops::RangeInclusive<usize>,
) {
    setting_row(ui, title, description, |ui| {
        let numeric_width = 80.0;
        let track_width = (bounded_control_width(ui, SETTINGS_CONTROL_WIDTH)
            - numeric_width
            - ui.spacing().item_spacing.x)
            .max(40.0);
        controls::slider(ui, value, range.clone(), track_width);
        ui.add_sized(
            [numeric_width, SETTINGS_ROW_HEIGHT],
            egui::DragValue::new(value).range(range).speed(1.0),
        );
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
        let numeric_width = 80.0;
        let track_width = (bounded_control_width(ui, SETTINGS_CONTROL_WIDTH)
            - numeric_width
            - ui.spacing().item_spacing.x)
            .max(40.0);
        controls::slider(ui, value, range.clone(), track_width);
        ui.add_sized(
            [numeric_width, SETTINGS_ROW_HEIGHT],
            egui::DragValue::new(value).range(range).speed(0.01),
        );
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

fn setting_path_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let mut spec = SettingSpec::new(title, description);
    spec.control_width = 460.0;
    setting_spec_row(ui, spec, add_control);
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
    // Reserve a paint slot before the row's controls so the highlight stays behind them.
    let highlight = ui.painter().add(egui::Shape::Noop);
    let response = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(0.0, SETTINGS_ROW_PAD_Y))
        .show(ui, |ui| {
            responsive_settings_row(
                ui,
                spec.control_width,
                |ui| setting_text(ui, spec.title, spec.description, spec.restart_requirement),
                |ui| {
                    let response = ui.scope(add_control).response;
                    setting_row_help(
                        response,
                        &setting_help(spec.description, spec.restart_requirement),
                    );
                },
            );
        })
        .response;
    controls::row_divider(ui, response.rect, fluent_palette(ui));
    let search_matched = ACTIVE_SEARCH_FOCUS.with(|slot| {
        slot.borrow().as_ref().and_then(|(query, at)| {
            let elapsed = at.elapsed().as_secs_f32();
            (elapsed <= 3.0 && (spec.title.contains(query) || query.contains(spec.title)))
                .then_some(elapsed)
        })
    });
    if let Some(elapsed) = search_matched {
        let palette = fluent_palette(ui);
        if elapsed < 0.12 {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        ui.painter().set(
            highlight,
            egui::epaint::RectShape::new(
                response.rect,
                egui::Rounding::same(palette.control_radius),
                palette.row_hover,
                Stroke::new(1.5_f32, palette.accent),
            ),
        );
    } else if ui.rect_contains_pointer(response.rect) {
        let palette = fluent_palette(ui);
        ui.painter().set(
            highlight,
            egui::Shape::rect_filled(response.rect, palette.control_radius, palette.row_hover),
        );
    }
    setting_row_help(
        response,
        &setting_help(spec.description, spec.restart_requirement),
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
            .width(bounded_control_width(ui, SETTINGS_CONTROL_WIDTH))
            .wrap()
            .show_settings(ui, add_options);
    });
}

#[path = "ui_input.rs"]
mod input;
use input::{custom_shortcuts_ui, input_habits_ui, input_tools_ui};

#[derive(Clone, Copy)]
struct CandidatePreviewColors {
    window: Color32,
    header: Color32,
    border: Color32,
    divider: Color32,
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
            selected: parse_skin_color(&skin.selected_bg, palette.nav_selected),
            selected_border: parse_skin_color(&skin.selected_border, palette.accent),
            text: parse_skin_color(&skin.text, palette.text),
            muted: parse_skin_color(&skin.muted_text, palette.muted),
            selected_text: parse_skin_color(&skin.selected_text, palette.accent_text),
            selected_muted: parse_skin_color(&skin.selected_muted_text, palette.accent_text),
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
        selected: palette.nav_selected,
        selected_border: palette.accent,
        text: palette.text,
        muted: palette.muted,
        selected_text: palette.accent_text,
        selected_muted: palette.accent_text,
        chip: palette.surface_alt,
        chip_border: palette.border,
        chip_text: palette.text,
    }
}

#[path = "ui_lexicons.rs"]
mod lexicons;
use lexicons::{extension_lexicon_controls, lexicon_learning_controls, user_lexicon_controls};

#[path = "ui_compatibility.rs"]
mod compatibility;
use compatibility::{compatibility_ui, game_test_wizard_ui};

fn sub_page_tabs<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    items: &[(T, &str)],
) -> bool {
    let previous = *current;
    let palette = fluent_palette(ui);
    let animation_id = ui.auto_id_with("settings_segment_highlight");
    controls::tabs_frame(palette).show(ui, |ui| {
        let background = (palette.style == UiDesignStyle::MacosBento)
            .then(|| ui.painter().add(egui::Shape::Noop));
        let mut selected_rect = None;
        ui.horizontal_wrapped(|ui| {
            for &(value, label) in items {
                let selected = *current == value;
                let response = ui.add(button(label, ButtonKind::Tab(selected)));
                if response.clicked() {
                    *current = value;
                    selected_rect = Some(response.rect);
                } else if selected {
                    selected_rect = Some(response.rect);
                }
            }
        });
        if let (Some(slot), Some(rect)) = (background, selected_rect) {
            controls::paint_segment(ui, slot, animation_id, rect, palette);
        }
    });
    *current != previous
}

fn appearance_page_tabs(ui: &mut egui::Ui, app: &mut SettingsApp) {
    app.reset_section_scroll |= sub_page_tabs(
        ui,
        &mut app.appearance_page,
        &[
            (AppearanceSettingsPage::Layout, "布局与显示"),
            (AppearanceSettingsPage::Theme, "字体与主题"),
            (AppearanceSettingsPage::Advanced, "高级标记"),
        ],
    );
}

fn lexicon_page_tabs(ui: &mut egui::Ui, app: &mut SettingsApp) {
    app.reset_section_scroll |= sub_page_tabs(
        ui,
        &mut app.lexicon_page,
        &[
            (LexiconSettingsPage::Learning, "学习策略"),
            (LexiconSettingsPage::User, "用户词条"),
        ],
    );
}

fn active_tool_page(app: &SettingsApp) -> Option<ToolSettingsPage> {
    Some(match app.active_section {
        SettingsSection::Screenshot => ToolSettingsPage::Screenshot,
        SettingsSection::Ocr => ToolSettingsPage::Ocr,
        SettingsSection::Clipboard => ToolSettingsPage::Clipboard,
        SettingsSection::Translation => ToolSettingsPage::Translation,
        SettingsSection::Handwrite => ToolSettingsPage::Handwrite,
        SettingsSection::Tools => ToolSettingsPage::Commands,
        _ => return None,
    })
}

fn system_page_tabs(ui: &mut egui::Ui, app: &mut SettingsApp) {
    app.reset_section_scroll |= sub_page_tabs(
        ui,
        &mut app.system_page,
        &[
            (SystemSettingsPage::General, "常规"),
            (SystemSettingsPage::Privacy, "隐私与数据"),
            (SystemSettingsPage::Advanced, "高级调优"),
            (SystemSettingsPage::About, "关于"),
        ],
    );
}

fn input_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    input_experience_ui(ui, app);
}

fn tools_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    tool_hotkey_link(ui, app);
    match app.tool_page {
        ToolSettingsPage::Handwrite => handwrite_settings_ui(ui, app),
        ToolSettingsPage::Commands => command_settings_ui(ui, app),
        ToolSettingsPage::Clipboard => clipboard_settings_ui(ui, app),
        ToolSettingsPage::Screenshot => screenshot_page_ui(ui, app),
        ToolSettingsPage::Ocr => {
            if resolve_desktop_tool("ocr").is_none() {
                inline_notice(
                    ui,
                    StatusTone::Warning,
                    "当前安装未包含 OCR 组件。安装 OCR 集合包后可用。",
                );
            } else {
                ocr_page_ui(ui, app);
            }
        }
        ToolSettingsPage::Translation => {
            if !translation_available() {
                inline_notice(
                    ui,
                    StatusTone::Warning,
                    "未检测到翻译服务；可先设置 HY-MT2 程序路径，再检测联动状态。",
                );
            }
            translation_page_ui(ui, app);
        }
    }
}

fn system_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    match app.system_page {
        SystemSettingsPage::General => {
            section_panel(ui, "状态提示", |ui| {
                setting_toggle(
                    ui,
                    "输入状态提示",
                    "显示中英、全角和标点等输入状态。",
                    &mut app.model.show_status_notifications,
                );
            });
            general_notifications_ui(ui, app);
            system_keyboard_ui(ui, app);
        }
        SystemSettingsPage::About => section_panel(ui, "关于开心输入法", |ui| {
            ui.heading("开心输入法");
            ui.label(format!("版本 {}", env!("CARGO_PKG_VERSION")));
            ui.label("本地优先的中文输入法");
            ui.label("配置、用户词库和学习数据保存在本机。");
        }),
        SystemSettingsPage::Privacy => privacy_data_ui(ui, app),
        SystemSettingsPage::Diagnostics => diagnostics_page_ui(ui, app),
        SystemSettingsPage::Advanced => advanced_page_ui(ui, app),
    }
}

fn input_experience_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    input_habits_ui(ui, &mut app.model);
    section_panel(ui, "输出方式", |ui| {
        setting_toggle(
            ui,
            "默认繁体输出",
            "开启后候选输出为繁体；也可使用下方快捷键临时切换。",
            &mut app.model.traditional_output,
        );
    });
    ui.add_space(2.0);
    input_tools_ui(ui, app);
}

fn candidate_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp, request_real_preview: &mut bool) {
    if let Some(selected_style) = candidate_appearance_ui(
        ui,
        &mut app.model,
        &mut app.status,
        &app.available_skins,
        &app.available_chinese_fonts,
        request_real_preview,
        app.appearance_page,
    ) {
        app.queue_design_style_change(
            selected_style,
            Some(format!(
                "已切换设置页与工具箱视觉风格：{}",
                selected_style.label()
            )),
        );
        ui.ctx().request_repaint();
    }
}

fn lexicon_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    match app.lexicon_page {
        LexiconSettingsPage::Learning => {
            section_panel(ui, "学习策略", |ui| lexicon_learning_controls(ui, app))
        }
        LexiconSettingsPage::User => {
            section_panel(ui, "用户词条与屏蔽词", |ui| {
                user_lexicon_controls(ui, app)
            });
            custom_shortcuts_ui(ui, app);
        }
        LexiconSettingsPage::Extensions => {
            section_panel(ui, "扩展词库", |ui| extension_lexicon_controls(ui, app))
        }
    }
}

fn screenshot_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    screenshot_settings_ui(ui, app);
}

fn diagnostics_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    diagnostics_ui(ui, app);
}

fn advanced_page_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    responsive_settings_columns(
        ui,
        "advanced_parameters",
        app,
        advanced_rank_ui,
        advanced_engine_tuning_ui,
    );
}

#[path = "ui_advanced.rs"]
mod advanced;
use advanced::{advanced_engine_tuning_ui, advanced_rank_ui};

#[path = "ui_diagnostics.rs"]
mod diagnostics;
use diagnostics::*;
pub(super) use diagnostics::{
    export_diagnostic_package_to, export_performance_log_to, DiagnosticsSnapshot,
};

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

fn system_keyboard_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
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
}

fn command_help_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    ui.add_space(10.0);
    section_panel(ui, "VV 命令速查", |ui| {
        let palette = fluent_palette(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new("筛选").small().color(palette.muted));
            ui.add_sized(
                [260.0, 26.0],
                text_field(
                    TextEdit::singleline(&mut app.vv_command_filter).hint_text("mail / rq / md"),
                ),
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

fn command_settings_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "直输命令与符号", |ui| {
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
    });
    command_help_ui(ui, app);
}

fn handwrite_settings_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let mut open_handwrite = false;
    section_panel(ui, "手写查字", |ui| {
        tool_row(
            ui,
            "手",
            "手写查字窗口",
            "打开独立手写画布，识别后可复制或直接粘贴候选字到当前焦点窗口。",
            |ui| {
                if outline_button(ui, "打开")
                    .on_hover_text("打开手写查字")
                    .clicked()
                {
                    open_handwrite = true;
                }
            },
        );
        setting_row(
            ui,
            "直输调用命令",
            "在拼音输入状态下输入 vv hw、vv handwrite 或 vv sx 即可调出手写窗口。",
            |ui| {
                status_badge(ui, StatusTone::Info, "vv hw / vv sx");
            },
        );
        setting_row(
            ui,
            "画布快捷键",
            "Enter 粘贴选中字 · Ctrl+C 复制 · 数字键 1–9 快粘 · Backspace/Ctrl+Z 撤销一笔 · Esc 清空。",
            |ui| {
                status_badge(ui, StatusTone::Success, "本地离线识别");
            },
        );
    });
    if open_handwrite {
        app.open_handwrite();
    }
}

fn tool_hotkey_link(ui: &mut egui::Ui, app: &mut SettingsApp) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("工具启动快捷键统一在“快捷键”页面设置。")
                .small()
                .color(fluent_palette(ui).muted),
        );
        if ui.link("设置快捷键").clicked() {
            app.active_section = SettingsSection::Hotkeys;
            app.reset_section_scroll = true;
        }
    });
    ui.add_space(8.0);
}
