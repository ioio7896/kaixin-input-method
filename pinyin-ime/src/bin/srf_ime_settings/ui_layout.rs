use super::*;

const SETTINGS_TEXT_MIN_WIDTH: f32 = 220.0;
const SETTINGS_COLUMN_GAP: f32 = 12.0;

// Allocate columns only when both fit. Child UIs grow to the actual wrapped
// content height, and controls can wrap without painting into the text column.
pub(super) fn responsive_settings_row(
    ui: &mut egui::Ui,
    preferred_control_width: f32,
    controls_first: bool,
    add_text: impl FnOnce(&mut egui::Ui),
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let total_width = ui.available_width().max(1.0);
    let gap = ui.spacing().item_spacing.x.max(SETTINGS_COLUMN_GAP);
    let control_width = preferred_control_width.min(total_width);
    let text_width = total_width - control_width - gap;
    let text = |ui: &mut egui::Ui, width: f32| {
        ui.set_width(width);
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing.y = ui.spacing().item_spacing.y.max(3.0);
        add_text(ui);
    };
    let controls = |ui: &mut egui::Ui, width: f32| {
        ui.set_width(width);
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing.x = ui.spacing().item_spacing.x.max(6.0);
        ui.spacing_mut().item_spacing.y = ui.spacing().item_spacing.y.max(6.0);
        add_control(ui);
    };
    if text_width < SETTINGS_TEXT_MIN_WIDTH {
        ui.vertical(|ui| {
            text(ui, total_width);
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(
                egui::vec2(total_width, SETTINGS_ROW_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                |ui| controls(ui, total_width),
            );
        });
    } else {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let add_text_column = |ui: &mut egui::Ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(text_width, SETTINGS_ROW_HEIGHT),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| text(ui, text_width),
                );
            };
            let add_control_column = |ui: &mut egui::Ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(control_width, SETTINGS_ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                    |ui| controls(ui, control_width),
                );
            };
            if controls_first {
                add_control_column(ui);
                add_text_column(ui);
            } else {
                add_text_column(ui);
                add_control_column(ui);
            }
        });
    }
}

pub(super) fn bounded_control_width(ui: &egui::Ui, preferred: f32) -> f32 {
    preferred.min(ui.available_width().max(1.0))
}
