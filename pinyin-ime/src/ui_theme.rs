//! Shared visual language for the egui-based companion tools.

use egui::{Color32, FontId, Frame, Margin, RichText, Stroke, TextStyle, Ui};

pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 16.0;
pub const SPACE_XL: f32 = 24.0;
pub const CONTROL_HEIGHT: f32 = 32.0;
pub const CONTROL_RADIUS: f32 = 6.0;
pub const CARD_RADIUS: f32 = 8.0;
pub const FLOATING_RADIUS: f32 = 10.0;

#[derive(Clone, Copy)]
pub struct UiPalette {
    pub app_bg: Color32,
    pub nav_bg: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub control_bg: Color32,
    pub control_hover: Color32,
    pub border: Color32,
    pub border_subtle: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub success: Color32,
    pub success_soft: Color32,
}

impl UiPalette {
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        if visuals.dark_mode {
            Self {
                app_bg: Color32::from_rgb(30, 32, 34),
                nav_bg: Color32::from_rgb(26, 28, 30),
                surface: Color32::from_rgb(42, 44, 46),
                surface_alt: Color32::from_rgb(36, 38, 40),
                control_bg: Color32::from_rgb(48, 50, 52),
                control_hover: Color32::from_rgb(55, 58, 60),
                border: Color32::from_rgb(70, 73, 76),
                border_subtle: Color32::from_rgb(55, 58, 60),
                text: Color32::from_rgb(245, 247, 246),
                muted: Color32::from_rgb(178, 185, 182),
                accent: Color32::from_rgb(76, 209, 151),
                accent_soft: Color32::from_rgb(35, 69, 56),
                accent_text: Color32::from_rgb(8, 30, 22),
                danger: Color32::from_rgb(255, 112, 112),
                danger_soft: Color32::from_rgb(68, 36, 36),
                success: Color32::from_rgb(76, 209, 151),
                success_soft: Color32::from_rgb(28, 58, 48),
            }
        } else {
            Self {
                app_bg: Color32::from_rgb(244, 247, 246),
                nav_bg: Color32::from_rgb(237, 242, 240),
                surface: Color32::from_rgb(253, 254, 254),
                surface_alt: Color32::from_rgb(241, 246, 244),
                control_bg: Color32::WHITE,
                control_hover: Color32::from_rgb(240, 248, 245),
                border: Color32::from_rgb(202, 214, 209),
                border_subtle: Color32::from_rgb(220, 229, 225),
                text: Color32::from_rgb(29, 40, 36),
                muted: Color32::from_rgb(96, 112, 105),
                accent: Color32::from_rgb(0, 137, 110),
                accent_soft: Color32::from_rgb(218, 241, 234),
                accent_text: Color32::WHITE,
                danger: Color32::from_rgb(190, 45, 45),
                danger_soft: Color32::from_rgb(255, 235, 235),
                success: Color32::from_rgb(0, 137, 110),
                success_soft: Color32::from_rgb(226, 246, 240),
            }
        }
    }
}

pub fn apply_tool_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let palette = UiPalette::from_visuals(&style.visuals);
    let radius = egui::Rounding::same(CONTROL_RADIUS);
    style.spacing.item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
    style.spacing.button_padding = egui::vec2(SPACE_MD, 6.0);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style.visuals.window_fill = palette.app_bg;
    style.visuals.panel_fill = palette.app_bg;
    style.visuals.extreme_bg_color = palette.control_bg;
    style.visuals.faint_bg_color = palette.surface_alt;
    style.visuals.widgets.inactive.bg_fill = palette.control_bg;
    style.visuals.widgets.hovered.bg_fill = palette.control_hover;
    style.visuals.widgets.active.bg_fill = palette.accent_soft;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.border);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    style.visuals.widgets.inactive.rounding = radius;
    style.visuals.widgets.hovered.rounding = radius;
    style.visuals.widgets.active.rounding = radius;
    style.visuals.widgets.open.rounding = radius;
    style.visuals.selection.bg_fill = palette.accent_soft;
    style.visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    ctx.set_style(style);
}

pub fn surface_card(ui: &mut Ui, selected: bool, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let palette = UiPalette::from_visuals(ui.visuals());
    Frame::none()
        .fill(if selected {
            palette.accent_soft
        } else {
            palette.surface
        })
        .stroke(Stroke::new(
            1.0,
            if selected {
                palette.accent
            } else {
                palette.border_subtle
            },
        ))
        .rounding(CARD_RADIUS)
        .inner_margin(Margin::symmetric(SPACE_MD, 10.0))
        .show(ui, add)
        .response
}

pub fn status_pill(ui: &mut Ui, text: &str, danger: bool) {
    let palette = UiPalette::from_visuals(ui.visuals());
    let (fill, color) = if danger {
        (palette.danger_soft, palette.danger)
    } else {
        (palette.success_soft, palette.success)
    };
    Frame::none()
        .fill(fill)
        .rounding(FLOATING_RADIUS)
        .inner_margin(Margin::symmetric(10.0, SPACE_XS))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
}

pub fn empty_state(ui: &mut Ui, symbol: &str, title: &str, hint: &str) {
    let palette = UiPalette::from_visuals(ui.visuals());
    ui.vertical_centered(|ui| {
        ui.add_space(SPACE_XL * 2.0);
        ui.label(RichText::new(symbol).size(32.0).color(palette.accent));
        ui.add_space(SPACE_SM);
        ui.label(RichText::new(title).strong().size(17.0).color(palette.text));
        ui.add_space(SPACE_XS);
        ui.label(RichText::new(hint).color(palette.muted));
    });
}
