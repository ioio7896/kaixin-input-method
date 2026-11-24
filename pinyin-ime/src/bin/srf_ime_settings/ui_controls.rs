use super::*;
use egui::{
    epaint::Shadow, pos2, vec2, Align2, Painter, Rect, Response, Sense, WidgetInfo, WidgetType,
};
use UiDesignStyle::*;

fn shadow(y: f32, blur: f32, color: Color32) -> Shadow {
    Shadow {
        offset: vec2(0.0, y),
        blur,
        spread: 0.0,
        color,
    }
}

fn horizontal_line(painter: &Painter, rect: Rect, y: f32, inset: f32, color: Color32) {
    if rect.width() > 2.0 * inset {
        painter.hline(
            rect.left() + inset..=rect.right() - inset,
            y,
            Stroke::new(1.0_f32, color),
        );
    }
}

fn focus_ring(painter: &Painter, rect: Rect, rounding: f32, color: Color32, response: &Response) {
    if response.has_focus() {
        painter.rect_stroke(rect.shrink(0.75), rounding, Stroke::new(1.5_f32, color));
    }
}

pub(super) fn switch(ui: &mut egui::Ui, label: &str, value: &mut bool) -> Response {
    let p = fluent_palette(ui);
    let size = match p.style {
        TactileIndustrial => vec2(38.0, 20.0),
        Win11Mica => vec2(40.0, 20.0),
        MacosBento => vec2(42.0, 24.0),
        LinearDark => vec2(36.0, 18.0),
        OrientalInk => vec2(38.0, 20.0),
    };
    let (bounds, mut response) =
        ui.allocate_exact_size(vec2(size.x, SETTINGS_ROW_HEIGHT), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *value, label));
    if !ui.is_rect_visible(bounds) {
        return response;
    }

    let painter = ui.painter();
    let track = Rect::from_center_size(bounds.center(), size).shrink(0.5);
    let progress = if p.style == TactileIndustrial {
        if *value {
            1.0
        } else {
            0.0
        }
    } else {
        ui.ctx().animate_bool_with_time(response.id, *value, 0.13)
    };
    let knob_center = |inset: f32| {
        let x = egui::lerp(track.left() + inset..=track.right() - inset, progress);
        pos2(x, track.center().y)
    };
    let accent = if response.hovered() {
        p.accent_hover
    } else {
        p.accent
    };
    let rounding = match p.style {
        TactileIndustrial => {
            painter.rect(track, 2.0, p.plate_bg, Stroke::new(1.0_f32, p.plate_border));
            horizontal_line(painter, track, track.bottom() - 1.0, 2.0, p.bevel_highlight);
            let led = Rect::from_center_size(
                pos2(track.right() - 4.0, track.center().y),
                vec2(2.0, 10.0),
            );
            painter.rect_filled(
                led.expand(1.0),
                1.0,
                if *value {
                    accent.gamma_multiply(0.32)
                } else {
                    p.plate_bg
                },
            );
            painter.rect_filled(led, 0.5, if *value { accent } else { p.border });
            let knob_x = if *value {
                track.right() - 13.0
            } else {
                track.left() + 9.0
            };
            let knob =
                Rect::from_center_size(pos2(knob_x, track.center().y - 0.5), vec2(14.0, 15.0));
            painter.rect_filled(knob.translate(vec2(0.0, 1.0)), 1.5, p.bevel_shadow);
            painter.rect(
                knob,
                1.5,
                if response.hovered() {
                    p.control_hover
                } else {
                    p.control_bg
                },
                Stroke::new(1.0_f32, p.plate_border),
            );
            horizontal_line(painter, knob, knob.top() + 1.5, 2.0, p.bevel_highlight);
            for x in [-2.0, 2.0] {
                painter.vline(
                    knob.center().x + x,
                    knob.center().y - 3.5..=knob.center().y + 3.5,
                    Stroke::new(1.0_f32, p.muted),
                );
            }
            2.0
        }
        Win11Mica => {
            let fill = if *value { accent } else { Color32::TRANSPARENT };
            let edge = if *value { accent } else { p.muted };
            painter.rect(track, 10.0, fill, Stroke::new(1.0_f32, edge));
            let radius = if response.hovered() { 6.6 } else { 5.3 };
            painter.circle_filled(
                knob_center(9.5),
                radius,
                if *value { p.accent_text } else { p.muted },
            );
            10.0
        }
        MacosBento => {
            painter.rect_filled(
                track,
                12.0,
                if *value { p.success } else { p.border_subtle },
            );
            let center = knob_center(11.25);
            let knob = Rect::from_center_size(center, vec2(19.0, 19.0));
            painter.add(shadow(1.0, 3.0, p.shadow_color).as_shape(knob, 10.0));
            painter.circle(
                center,
                9.5,
                Color32::WHITE,
                Stroke::new(0.6_f32, p.border_subtle),
            );
            12.0
        }
        LinearDark => {
            painter.rect(track, 5.0, p.app_bg, Stroke::new(1.0_f32, p.border));
            let knob = Rect::from_center_size(knob_center(8.5), vec2(13.0, 11.0));
            if *value {
                painter.add(shadow(0.0, 7.0, accent.gamma_multiply(0.48)).as_shape(knob, 3.0));
            }
            painter.rect(
                knob,
                3.0,
                if *value { p.info_soft } else { p.control_hover },
                Stroke::new(1.0_f32, if *value { accent } else { p.border }),
            );
            5.0
        }
        OrientalInk => {
            painter.rect(track, 3.0, p.surface_alt, Stroke::new(1.0_f32, p.border));
            let center = knob_center(9.0);
            let seal = Rect::from_center_size(center, vec2(14.0, 14.0));
            painter.rect_filled(seal, 1.0, if *value { accent } else { p.muted });
            if *value {
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    "启",
                    FontId::proportional(10.0),
                    p.accent_text,
                );
            } else {
                painter.vline(
                    center.x,
                    center.y - 4.0..=center.y + 4.0,
                    Stroke::new(1.0_f32, p.surface),
                );
            }
            3.0
        }
    };
    focus_ring(painter, track.expand(2.0), rounding, p.accent, &response);
    response
}

#[derive(Clone, Copy)]
pub(super) enum ButtonKind {
    Primary,
    Secondary,
    Danger,
    Compact(bool),
    Key(bool),
    Tab(bool),
}

pub(super) fn button(label: &str, kind: ButtonKind) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| {
        let p = fluent_palette(ui);
        let (min_width, height) = match kind {
            ButtonKind::Primary => (104.0, 32.0),
            ButtonKind::Secondary | ButtonKind::Danger => {
                (SETTINGS_BUTTON_WIDTH, SETTINGS_ROW_HEIGHT)
            }
            ButtonKind::Compact(_) => (42.0, 24.0),
            ButtonKind::Key(_) => (34.0, 24.0),
            ButtonKind::Tab(_) => (72.0, 27.0),
        };
        let font = if matches!(kind, ButtonKind::Key(_)) {
            FontId::monospace(12.0)
        } else if matches!(kind, ButtonKind::Compact(_)) {
            FontId::proportional(SETTINGS_FONT_SMALL)
        } else {
            FontId::proportional(SETTINGS_FONT_BODY)
        };
        let primary = matches!(kind, ButtonKind::Primary) && ui.is_enabled();
        let danger = matches!(kind, ButtonKind::Danger) && ui.is_enabled();
        let selected = matches!(
            kind,
            ButtonKind::Key(true) | ButtonKind::Tab(true) | ButtonKind::Compact(true)
        );
        let text_color = if primary {
            p.accent_text
        } else if danger {
            p.danger
        } else {
            p.text
        };
        let mut text = RichText::new(label).font(font).color(text_color);
        if matches!(kind, ButtonKind::Primary) || danger || selected {
            text = text.strong();
        }
        let padding = if matches!(kind, ButtonKind::Key(_) | ButtonKind::Compact(_)) {
            6.0
        } else {
            9.0
        };
        let width_limit = ui.available_width().max(1.0);
        let galley = egui::WidgetText::from(text).into_galley(
            ui,
            Some(egui::TextWrapMode::Wrap),
            (width_limit - padding * 2.0).max(1.0),
            TextStyle::Button,
        );
        let size = vec2(
            (galley.size().x + padding * 2.0)
                .max(min_width)
                .min(width_limit),
            (galley.size().y + 8.0).max(height),
        );
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| match kind {
            ButtonKind::Key(on) => {
                WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label)
            }
            ButtonKind::Tab(on) | ButtonKind::Compact(on) => {
                WidgetInfo::selected(WidgetType::SelectableLabel, ui.is_enabled(), on, label)
            }
            _ => WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label),
        });
        if ui.is_rect_visible(rect) {
            let face = button_face(
                ui,
                rect.shrink(0.75),
                &response,
                kind,
                p,
                primary,
                danger,
                selected,
            );
            ui.painter()
                .galley(face.center() - galley.size() * 0.5, galley, text_color);
            focus_ring(
                ui.painter(),
                rect,
                p.control_radius,
                if danger { p.danger } else { p.accent },
                &response,
            );
        }
        response
    }
}

#[allow(clippy::too_many_arguments)]
fn button_face(
    ui: &egui::Ui,
    rect: Rect,
    response: &Response,
    kind: ButtonKind,
    p: FluentPalette,
    primary: bool,
    danger: bool,
    selected: bool,
) -> Rect {
    let painter = ui.painter();
    let down = response.is_pointer_button_down_on();
    let hover = response.hovered();
    let tab = matches!(kind, ButtonKind::Tab(_));
    let fill = if primary {
        match (down, hover) {
            (true, _) => p.accent_pressed,
            (false, true) => p.accent_hover,
            _ => p.accent,
        }
    } else if danger {
        if down || hover {
            p.danger
                .gamma_multiply(if p.dark_mode { 0.30 } else { 0.18 })
        } else {
            p.danger_soft
        }
    } else if down || hover {
        p.control_hover
    } else if selected {
        p.info_soft
    } else {
        p.control_bg
    };

    match p.style {
        TactileIndustrial => {
            let recess = if down {
                1.5
            } else if selected {
                1.0
            } else {
                0.0
            };
            let face = Rect::from_min_max(rect.min, rect.max - vec2(0.0, 2.5))
                .translate(vec2(0.0, recess));
            if !down {
                painter.rect_filled(rect, p.control_radius, p.plate_border);
            }
            let edge = if danger { p.danger } else { p.plate_border };
            painter.rect(face, p.control_radius, fill, Stroke::new(1.2_f32, edge));
            horizontal_line(painter, face, face.top() + 1.5, 3.0, p.bevel_highlight);
            if selected {
                horizontal_line(painter, face, face.bottom() - 2.5, 4.0, p.accent);
            }
            face
        }
        Win11Mica => {
            let edge = if primary {
                fill
            } else if danger {
                p.danger
            } else {
                p.border_subtle
            };
            painter.rect(rect, p.control_radius, fill, Stroke::new(1.0_f32, edge));
            horizontal_line(painter, rect, rect.top() + 1.0, 4.0, p.bevel_highlight);
            horizontal_line(painter, rect, rect.bottom() - 1.0, 4.0, p.bevel_shadow);
            if tab && selected {
                underline(painter, rect, p.accent);
            }
            rect
        }
        MacosBento => {
            let radius = if tab { 13.0 } else { p.control_radius };
            if !tab && !down {
                painter.add(shadow(1.5, 5.0, p.shadow_color).as_shape(rect, radius));
            }
            if !tab || (hover && !selected) {
                let stroke = if danger {
                    Stroke::new(1.0_f32, p.danger)
                } else if !primary && !tab {
                    Stroke::new(1.0_f32, p.border_subtle)
                } else {
                    Stroke::NONE
                };
                painter.rect(rect, radius, fill, stroke);
            }
            rect
        }
        LinearDark => {
            let glow_color = if danger { p.danger } else { p.accent };
            if hover {
                painter.add(
                    shadow(0.0, 8.0, glow_color.gamma_multiply(0.30))
                        .as_shape(rect, p.control_radius),
                );
            }
            if !tab || selected || hover {
                painter.rect(
                    rect,
                    p.control_radius,
                    fill,
                    Stroke::new(
                        1.0_f32,
                        if danger {
                            p.danger
                        } else if hover || selected {
                            p.accent
                        } else {
                            p.border
                        },
                    ),
                );
                horizontal_line(painter, rect, rect.top() + 1.0, 4.0, p.bevel_highlight);
            }
            rect
        }
        OrientalInk => {
            let edge = if danger {
                p.danger
            } else if primary || selected {
                p.accent
            } else {
                p.border
            };
            if down {
                painter.add(
                    shadow(0.0, 7.0, edge.gamma_multiply(0.30)).as_shape(rect, p.control_radius),
                );
            }
            painter.rect(rect, p.control_radius, fill, Stroke::new(1.0_f32, edge));
            if primary {
                painter.rect_stroke(
                    rect.shrink(2.0),
                    1.0,
                    Stroke::new(0.7_f32, p.bevel_highlight),
                );
            } else if selected {
                underline(painter, rect, edge);
            }
            rect
        }
    }
}

fn underline(painter: &Painter, rect: Rect, color: Color32) {
    let mark = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 2.0), vec2(18.0, 2.0));
    painter.rect_filled(mark, 1.0, color);
}

pub(super) fn tabs_frame(p: FluentPalette) -> egui::Frame {
    let frame = egui::Frame::none().inner_margin(egui::Margin::symmetric(4.0, 3.0));
    match p.style {
        TactileIndustrial => frame
            .fill(p.surface_alt)
            .stroke(Stroke::new(1.0_f32, p.border))
            .rounding(3.0),
        MacosBento => frame.fill(p.surface_alt).rounding(16.0),
        LinearDark => frame.fill(p.app_bg).rounding(4.0),
        OrientalInk | Win11Mica => frame,
    }
}

pub(super) fn paint_segment(
    ui: &egui::Ui,
    slot: egui::layers::ShapeIdx,
    id: egui::Id,
    target: Rect,
    p: FluentPalette,
) {
    let ease = |edge, value| ui.ctx().animate_value_with_time(id.with(edge), value, 0.14);
    let rect = Rect::from_min_max(
        pos2(ease("left", target.left()), ease("top", target.top())),
        pos2(
            ease("right", target.right()),
            ease("bottom", target.bottom()),
        ),
    )
    .shrink(0.75);
    let offset = shadow(1.5, 5.0, p.shadow_color).as_shape(rect, 13.0);
    ui.painter().set(
        slot,
        egui::Shape::Vec(vec![
            offset.into(),
            egui::Shape::rect_filled(rect, 13.0, p.surface),
        ]),
    );
}

pub(super) fn panel(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let p = fluent_palette(ui);
    let title = RichText::new(title)
        .strong()
        .size(SETTINGS_FONT_SECTION_TITLE);
    if p.style == Win11Mica {
        ui.label(title.clone().color(p.text));
    }
    let card = egui::Frame::none().fill(p.surface);
    let card = match p.style {
        TactileIndustrial => card
            .rounding(p.card_radius)
            .stroke(Stroke::new(p.border_width, p.border))
            .inner_margin(egui::Margin::same(12.0)),
        Win11Mica => card
            .rounding(p.card_radius)
            .stroke(Stroke::new(1.0_f32, p.border_subtle))
            .inner_margin(egui::Margin::symmetric(10.0, 4.0)),
        MacosBento => card
            .rounding(p.card_radius)
            .stroke(Stroke::new(1.0_f32, p.border_subtle))
            .shadow(shadow(3.0, 12.0, p.shadow_color))
            .inner_margin(egui::Margin::same(14.0)),
        LinearDark => card
            .rounding(p.card_radius)
            .stroke(Stroke::new(1.0_f32, p.border))
            .inner_margin(egui::Margin::same(10.0)),
        OrientalInk => card
            .rounding(p.card_radius)
            .stroke(Stroke::new(1.2_f32, p.border))
            .inner_margin(egui::Margin::same(13.0)),
    };
    let content_width = (ui.available_width() - card.total_margin().sum().x).max(1.0);
    let rect = card
        .show(ui, |ui| {
            ui.set_width(content_width);
            match p.style {
                TactileIndustrial => {
                    let mut strip = egui::Frame::none()
                        .fill(p.plate_bg)
                        .stroke(Stroke::new(1.0_f32, p.plate_border))
                        .rounding(2.0)
                        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                        .begin(ui);
                    strip
                        .content_ui
                        .set_min_width((content_width - 16.0).max(1.0));
                    strip.content_ui.horizontal(|ui| {
                        indicator(ui, p.accent, false);
                        if let Some(number) = ui.data_mut(|data| {
                            let number = data.get_temp::<usize>(module_counter())? + 1;
                            data.insert_temp(module_counter(), number);
                            Some(number)
                        }) {
                            ui.label(
                                RichText::new(format!("MOD // {number:02}"))
                                    .monospace()
                                    .size(10.0)
                                    .color(p.muted),
                            );
                        }
                        ui.label(title.clone().color(p.plate_text));
                    });
                    let strip = strip.end(ui).rect;
                    cut_line(
                        ui.painter(),
                        strip.expand2(vec2(0.0, 2.0)),
                        strip.bottom(),
                        p.border,
                        p.surface,
                    );
                }
                MacosBento => {
                    ui.label(title.clone().size(16.0).color(p.text));
                }
                LinearDark => {
                    ui.horizontal(|ui| {
                        ui.label(title.clone().color(p.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            indicator(ui, p.accent, false);
                        });
                    });
                }
                OrientalInk => {
                    ui.horizontal(|ui| {
                        let (seal, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(seal, 1.0, p.accent);
                        ui.painter().text(
                            seal.center(),
                            Align2::CENTER_CENTER,
                            "印",
                            FontId::proportional(10.0),
                            p.accent_text,
                        );
                        ui.label(title.clone().color(p.text));
                    });
                }
                Win11Mica => {}
            }
            if p.style != Win11Mica {
                ui.add_space(if p.style == MacosBento { 6.0 } else { 4.0 });
            }
            add_contents(ui);
        })
        .response
        .rect;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        match p.style {
            TactileIndustrial => {
                for x in [rect.left() + 6.0, rect.right() - 6.0] {
                    for y in [rect.top() + 6.0, rect.bottom() - 6.0] {
                        painter.circle_filled(pos2(x, y), 2.7, p.border);
                        let slot = Stroke::new(0.8_f32, p.plate_border);
                        painter.hline(x - 1.5..=x + 1.5, y, slot);
                        painter.vline(x, y - 1.5..=y + 1.5, slot);
                    }
                }
            }
            LinearDark => horizontal_line(painter, rect, rect.top() + 1.0, 5.0, p.bevel_highlight),
            OrientalInk => {
                painter.rect_stroke(
                    rect.shrink(3.5),
                    (p.card_radius - 1.5).max(0.0),
                    Stroke::new(0.7_f32, p.border_subtle),
                );
            }
            _ => {}
        }
    }
    ui.add_space(if p.style == MacosBento { 9.0 } else { 6.0 });
}

pub(super) fn module_counter() -> egui::Id {
    egui::Id::new("settings_module_counter")
}

fn indicator(ui: &mut egui::Ui, color: Color32, round: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 12.0), Sense::hover());
    let lamp = Rect::from_center_size(rect.center(), vec2(6.0, 6.0));
    let radius = if round { 3.0 } else { 1.0 };
    ui.painter()
        .rect_filled(lamp.expand(1.0), radius, color.gamma_multiply(0.38));
    ui.painter().rect_filled(lamp, radius, color);
}

fn cut_line(painter: &Painter, rect: Rect, y: f32, groove: Color32, reflection: Color32) {
    horizontal_line(painter, rect, y, 0.5, groove);
    horizontal_line(painter, rect, y + 1.0, 0.5, reflection);
}

pub(super) fn row_divider(ui: &egui::Ui, rect: Rect, p: FluentPalette) {
    let y = rect.bottom() + 1.0;
    match p.style {
        TactileIndustrial => cut_line(ui.painter(), rect, y, p.border_subtle, p.bevel_highlight),
        Win11Mica => horizontal_line(
            ui.painter(),
            rect.expand2(vec2(9.0, 0.0)),
            y,
            0.0,
            p.border_subtle,
        ),
        _ => {}
    }
}

pub(super) fn section_number(section: SettingsSection) -> usize {
    SettingsSection::ALL
        .iter()
        .position(|&item| item == section)
        .unwrap_or_default()
        + 1
}

pub(super) fn navigation(
    ui: &mut egui::Ui,
    section: SettingsSection,
    selected: bool,
    hint: String,
) -> Response {
    let p = fluent_palette(ui);
    let height = match p.style {
        LinearDark => 26.0,
        MacosBento => 32.0,
        _ => 30.0,
    };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let response = response.on_hover_text(hint);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            section.label(),
        )
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let painter = ui.painter();
    let hover = response.hovered();
    let (icon_x, text_x) = match p.style {
        TactileIndustrial => {
            if selected {
                painter.rect(
                    rect,
                    p.control_radius,
                    p.nav_hover,
                    Stroke::new(1.2_f32, p.plate_border),
                );
                horizontal_line(painter, rect, rect.top() + 1.5, 3.0, p.bevel_shadow);
                horizontal_line(painter, rect, rect.bottom() - 1.5, 3.0, p.bevel_highlight);
                let lamp = Rect::from_center_size(
                    pos2(rect.right() - 9.0, rect.center().y),
                    vec2(6.0, 6.0),
                );
                painter.rect_filled(lamp.expand(1.0), 1.0, p.plate_bg);
                painter.rect_filled(lamp, 0.5, p.accent);
            } else if hover {
                painter.rect_filled(rect, p.control_radius, p.nav_hover);
            }
            let plate =
                Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(20.0, 16.0));
            painter.rect_filled(plate, 1.5, p.plate_bg);
            painter.text(
                plate.center(),
                Align2::CENTER_CENTER,
                format!("{:02}", section_number(section)),
                FontId::monospace(10.0),
                p.plate_text,
            );
            (40.0, 56.0)
        }
        Win11Mica => {
            if selected || hover {
                painter.rect_filled(rect, p.control_radius, p.nav_hover);
            }
            if selected {
                let bar = Rect::from_center_size(
                    pos2(rect.left() + 3.5, rect.center().y),
                    vec2(3.0, 16.0),
                );
                painter.rect_filled(bar, 1.5, p.accent);
            }
            (22.0, 41.0)
        }
        MacosBento => {
            if selected {
                painter.add(shadow(1.5, 5.0, p.shadow_color).as_shape(rect, p.control_radius));
                painter.rect_filled(rect, p.control_radius, p.surface);
            } else if hover {
                painter.rect_filled(rect, p.control_radius, p.nav_hover);
            }
            (19.0, 37.0)
        }
        LinearDark => {
            if selected || hover {
                painter.rect_filled(rect, p.control_radius, p.nav_hover);
            }
            if selected {
                let bar = Rect::from_center_size(
                    pos2(rect.left() + 2.0, rect.center().y),
                    vec2(2.0, 16.0),
                );
                painter.add(shadow(0.0, 6.0, p.accent.gamma_multiply(0.5)).as_shape(bar, 1.0));
                painter.rect_filled(bar, 1.0, p.accent);
            }
            if selected || hover {
                for (index, key) in ["G", nav_key(section).name()].into_iter().enumerate() {
                    let badge = Rect::from_center_size(
                        pos2(rect.right() - 25.0 + index as f32 * 16.0, rect.center().y),
                        vec2(14.0, 16.0),
                    );
                    painter.rect(badge, 2.0, p.app_bg, Stroke::new(1.0_f32, p.border));
                    painter.text(
                        badge.center(),
                        Align2::CENTER_CENTER,
                        key,
                        FontId::monospace(10.0),
                        p.muted,
                    );
                }
            }
            (17.0, 33.0)
        }
        OrientalInk => {
            if selected || hover {
                painter.rect_filled(
                    rect.shrink2(vec2(4.0, 2.0)),
                    p.control_radius,
                    p.surface_alt,
                );
            }
            if selected {
                let x = rect.left() + 6.0;
                painter.vline(
                    x,
                    rect.top() + 4.0..=rect.bottom() - 4.0,
                    Stroke::new(1.5_f32, p.accent),
                );
                let stamp = pos2(rect.right() - 10.0, rect.center().y);
                painter.circle_stroke(stamp, 3.5, Stroke::new(0.8_f32, p.accent));
                painter.circle_filled(stamp, 1.7, p.accent);
            }
            (22.0, 40.0)
        }
    };
    let icon_color = if selected && matches!(p.style, MacosBento | LinearDark | OrientalInk) {
        p.accent
    } else {
        p.text
    };
    paint_settings_icon(
        ui,
        pos2(rect.left() + icon_x, rect.center().y),
        section.icon(),
        icon_color,
    );
    painter.text(
        pos2(rect.left() + text_x, rect.center().y),
        Align2::LEFT_CENTER,
        section.label(),
        FontId::proportional(SETTINGS_FONT_NAV_TITLE),
        p.text,
    );
    focus_ring(painter, rect, p.control_radius, p.accent, &response);
    response
}

fn nav_key(section: SettingsSection) -> egui::Key {
    use egui::Key;
    match section {
        SettingsSection::Input => Key::I,
        SettingsSection::Appearance => Key::A,
        SettingsSection::Lexicon => Key::D,
        SettingsSection::Learning => Key::L,
        SettingsSection::Hotkeys => Key::K,
        SettingsSection::Screenshot => Key::S,
        SettingsSection::Ocr => Key::O,
        SettingsSection::Clipboard => Key::C,
        SettingsSection::Translation => Key::T,
        SettingsSection::Handwrite => Key::H,
        SettingsSection::Diagnostics => Key::P,
        SettingsSection::Tools => Key::M,
        SettingsSection::Compatibility => Key::R,
        SettingsSection::System => Key::Y,
    }
}

pub(super) fn navigation_shortcut(ctx: &egui::Context) -> Option<SettingsSection> {
    let id = egui::Id::new("settings_linear_navigation");
    let prefix = ctx.data_mut(|data| data.remove_temp::<f64>(id));
    if ctx.wants_keyboard_input() || ctx.memory(|memory| memory.any_popup_open()) {
        return None;
    }
    let now = ctx.input(|input| input.time);
    if let Some(deadline) = prefix.filter(|&deadline| now < deadline) {
        if let Some(section) = ctx.input_mut(|input| {
            SettingsSection::ALL
                .into_iter()
                .find(|&section| input.consume_key(egui::Modifiers::NONE, nav_key(section)))
        }) {
            return Some(section);
        }
        ctx.data_mut(|data| data.insert_temp(id, deadline));
    }
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::G)) {
        ctx.data_mut(|data| data.insert_temp(id, now + 0.9));
    }
    None
}

pub(super) fn slider<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    width: f32,
) {
    let p = fluent_palette(ui);
    let start = range.start().to_f64();
    let end = range.end().to_f64();
    // Keep egui's drag, keyboard, clamping and accessibility behavior. Paint the rail and cap here.
    let response = ui
        .scope(|ui| {
            ui.set_opacity(0.0);
            ui.spacing_mut().slider_width = width;
            ui.add_sized(
                [width, SETTINGS_ROW_HEIGHT],
                Slider::new(value, range)
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Circle),
            )
        })
        .inner;
    let rect = response.rect;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let fraction = if start == end {
        0.5
    } else {
        ((value.to_f64() - start) / (end - start)).clamp(0.0, 1.0) as f32
    };
    // The native circle handle reserves the slider height / 2.5 at either end.
    let inset = (rect.height() / 2.5).min(rect.width() * 0.5);
    let y = rect.center().y;
    let left = rect.left() + inset;
    let right = rect.right() - inset;
    let center = pos2(egui::lerp(left..=right, fraction), y);
    let height = match p.style {
        TactileIndustrial => 5.0,
        Win11Mica => 3.0,
        MacosBento => 7.0,
        LinearDark => 2.0,
        OrientalInk => 1.5,
    };
    let painter = ui.painter();
    let accent = if response.hovered() {
        p.accent_hover
    } else {
        p.accent
    };
    let rail = Rect::from_min_max(pos2(left, y - height * 0.5), pos2(right, y + height * 0.5));
    let groove = match p.style {
        TactileIndustrial => p.plate_bg,
        OrientalInk => p.muted,
        _ => p.border,
    };
    painter.rect_filled(rail, height * 0.5, groove);
    painter.rect_filled(
        Rect::from_min_max(rail.min, pos2(center.x, rail.bottom())),
        height * 0.5,
        accent,
    );
    match p.style {
        TactileIndustrial => {
            for mark in 0..=10 {
                let x = egui::lerp(left..=right, mark as f32 / 10.0);
                let tick = if mark % 5 == 0 { 4.0 } else { 2.0 };
                painter.vline(x, y + 8.0..=y + 8.0 + tick, Stroke::new(0.8_f32, p.muted));
            }
            let cap = Rect::from_center_size(center, vec2(13.0, 17.0));
            painter.rect_filled(cap.translate(vec2(0.0, 1.5)), 2.0, p.bevel_shadow);
            painter.rect(cap, 2.0, p.control_bg, Stroke::new(1.0_f32, p.plate_border));
            horizontal_line(painter, cap, cap.top() + 1.5, 2.0, p.bevel_highlight);
            painter.vline(center.x, y - 5.0..=y + 5.0, Stroke::new(2.0_f32, accent));
        }
        Win11Mica => {
            painter.circle(center, 7.0, p.surface, Stroke::new(1.0_f32, p.border));
            painter.circle_filled(center, if response.hovered() { 5.0 } else { 3.7 }, accent);
        }
        MacosBento => {
            let knob = Rect::from_center_size(center, vec2(17.0, 17.0));
            painter.add(shadow(1.0, 4.0, p.shadow_color).as_shape(knob, 9.0));
            painter.circle(
                center,
                8.5,
                Color32::WHITE,
                Stroke::new(0.7_f32, p.border_subtle),
            );
        }
        LinearDark => {
            painter.circle_filled(center, 6.0, accent.gamma_multiply(0.24));
            painter.circle(center, 3.5, p.app_bg, Stroke::new(1.0_f32, accent));
        }
        OrientalInk => {
            painter.circle(center, 4.5, accent, Stroke::new(0.7_f32, p.surface));
        }
    }
    focus_ring(painter, rect, p.control_radius, p.accent, &response);
}

fn lcd_fill(p: FluentPalette) -> Color32 {
    if p.dark_mode {
        p.app_bg
    } else {
        Color32::from_rgb(220, 223, 210)
    }
}

fn recessed_edges(ui: &egui::Ui, rect: Rect, p: FluentPalette) {
    if ui.is_rect_visible(rect) {
        horizontal_line(ui.painter(), rect, rect.top() + 1.0, 3.0, p.bevel_shadow);
        horizontal_line(
            ui.painter(),
            rect,
            rect.bottom() - 1.0,
            3.0,
            p.bevel_highlight,
        );
    }
}

pub(super) fn text_field(edit: TextEdit<'_>) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| {
        let p = fluent_palette(ui);
        if p.style != TactileIndustrial {
            return ui.add(edit);
        }
        let previous_fill = ui.visuals().extreme_bg_color;
        ui.visuals_mut().extreme_bg_color = lcd_fill(p);
        let response = ui.add(edit);
        ui.visuals_mut().extreme_bg_color = previous_fill;
        recessed_edges(ui, response.rect, p);
        response
    }
}

pub(in super::super) trait SettingsComboBoxExt {
    fn show_settings<R>(
        self,
        ui: &mut egui::Ui,
        menu: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>>;
}

impl SettingsComboBoxExt for ComboBox {
    fn show_settings<R>(
        self,
        ui: &mut egui::Ui,
        menu: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>> {
        let p = fluent_palette(ui);
        if p.style != TactileIndustrial {
            return self.show_ui(ui, menu);
        }
        let fill = lcd_fill(p);
        let previous_widgets = ui.visuals().widgets.clone();
        let widgets = &mut ui.visuals_mut().widgets;
        for widget in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            widget.weak_bg_fill = fill;
            widget.bg_fill = fill;
        }
        let result = self.show_ui(ui, |ui| {
            ui.visuals_mut().widgets = previous_widgets.clone();
            menu(ui)
        });
        ui.visuals_mut().widgets = previous_widgets;
        recessed_edges(ui, result.response.rect, p);
        result
    }
}

pub(super) fn header_frame(p: FluentPalette) -> egui::Frame {
    let frame = egui::Frame::none().inner_margin(egui::Margin::symmetric(9.0, 4.0));
    match p.style {
        TactileIndustrial => frame
            .fill(p.plate_bg)
            .rounding(p.control_radius)
            .stroke(Stroke::new(1.0_f32, p.plate_border)),
        Win11Mica => egui::Frame::none(),
        MacosBento => frame
            .fill(p.surface)
            .rounding(p.card_radius)
            .stroke(Stroke::new(1.0_f32, p.border_subtle))
            .shadow(shadow(2.0, 8.0, p.shadow_color)),
        LinearDark => frame
            .fill(p.surface)
            .rounding(p.control_radius)
            .stroke(Stroke::new(1.0_f32, p.border)),
        OrientalInk => frame
            .fill(p.surface_alt)
            .rounding(p.control_radius)
            .stroke(Stroke::new(1.0_f32, p.border)),
    }
}

pub(super) fn page_title(ui: &mut egui::Ui, section: SettingsSection, p: FluentPalette) {
    let color = match p.style {
        TactileIndustrial => {
            indicator(ui, p.accent, false);
            ui.label(
                RichText::new(format!("{:02}", section_number(section)))
                    .monospace()
                    .size(13.0)
                    .color(p.plate_text),
            );
            p.plate_text
        }
        LinearDark => {
            ui.label(RichText::new(">").monospace().strong().color(p.accent));
            p.text
        }
        OrientalInk => {
            indicator(ui, p.accent, true);
            p.text
        }
        _ => p.text,
    };
    let size = match p.style {
        MacosBento => 23.0,
        LinearDark => 18.0,
        _ => SETTINGS_FONT_PAGE_TITLE,
    };
    ui.label(
        RichText::new(section.label())
            .strong()
            .size(size)
            .color(color),
    )
    .on_hover_text(section.hint());
}

/// Outer frame for the bottom action panel.
/// Always opaque edge-to-edge with zero outer_margin and no outer window stroke
/// so clear_color never leaks through rounded corners or margins on any style.
pub(super) fn footer_outer_frame(p: FluentPalette) -> egui::Frame {
    match p.style {
        MacosBento => egui::Frame::none()
            .fill(p.app_bg)
            // The capsule shadow extends seven points below its frame.
            .inner_margin(egui::Margin::symmetric(10.0, 8.0)),
        LinearDark => egui::Frame::none()
            .fill(p.app_bg)
            .inner_margin(egui::Margin::symmetric(16.0, 8.0)),
        OrientalInk => egui::Frame::none()
            .fill(p.surface_alt)
            .inner_margin(egui::Margin::symmetric(16.0, 8.0)),
        TactileIndustrial | Win11Mica => egui::Frame::none()
            .fill(p.command_bar)
            .inner_margin(egui::Margin::symmetric(16.0, 8.0)),
    }
}

pub(super) fn show_footer_container(
    ui: &mut egui::Ui,
    p: FluentPalette,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let panel_rect = ui.clip_rect();
    match p.style {
        MacosBento => {
            egui::Frame::none()
                .fill(p.command_bar)
                .stroke(Stroke::new(1.0_f32, p.border_subtle))
                .rounding(p.card_radius)
                .shadow(shadow(2.0, 10.0, p.shadow_color))
                .inner_margin(egui::Margin::symmetric(14.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add_contents(ui);
                });
        }
        _ => {
            let border_color = match p.style {
                Win11Mica => p.border_subtle,
                _ => p.border,
            };
            let y = ui.painter().round_to_pixel_center(panel_rect.top() + 0.5);
            ui.painter()
                .hline(panel_rect.x_range(), y, Stroke::new(1.0_f32, border_color));
            add_contents(ui);
        }
    }
}

pub(super) fn footer_readout(
    ui: &mut egui::Ui,
    p: FluentPalette,
    text: &str,
    color: Color32,
    dirty: bool,
) {
    match p.style {
        TactileIndustrial => {
            let frame = egui::Frame::none()
                .fill(p.plate_bg)
                .stroke(Stroke::new(1.0_f32, p.plate_border))
                .rounding(p.control_radius)
                .inner_margin(egui::Margin::symmetric(7.0, 3.0));
            let width = (ui.available_width() - frame.total_margin().sum().x).max(1.0);
            let rect = frame
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.horizontal_top(|ui| {
                        let lamp = if dirty {
                            p.accent
                        } else {
                            Color32::from_rgb(142, 213, 155)
                        };
                        indicator(ui, lamp, false);
                        ui.label(
                            RichText::new(if dirty { "DIRTY" } else { "READY" })
                                .monospace()
                                .strong()
                                .size(11.0)
                                .color(lamp),
                        );
                        footer_status(ui, text, Color32::from_rgb(201, 215, 188), true);
                    });
                })
                .response
                .rect;
            recessed_edges(ui, rect, p);
            let y = rect.bottom() - 5.0;
            for index in 0..((rect.width() - 14.0) / 4.0).max(0.0) as usize {
                let x = rect.left() + 7.0 + index as f32 * 4.0;
                ui.painter()
                    .circle_filled(pos2(x, y), 0.55, Color32::from_rgb(76, 91, 77));
            }
        }
        LinearDark | OrientalInk => {
            ui.horizontal_top(|ui| {
                if p.style == LinearDark {
                    ui.label(RichText::new(">").monospace().strong().color(p.accent));
                } else {
                    egui::Frame::none()
                        .fill(p.accent)
                        .rounding(1.0)
                        .inner_margin(egui::Margin::same(2.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(if dirty { "待存" } else { "本机" })
                                    .size(10.0)
                                    .color(p.accent_text),
                            );
                        });
                }
                footer_status(ui, text, color, p.style == LinearDark);
            });
        }
        _ => footer_status(ui, text, color, false),
    }
}
