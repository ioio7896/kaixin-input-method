use super::*;

const SETTINGS_TEXT_MIN_WIDTH: f32 = 180.0;
const SETTINGS_COLUMN_GAP: f32 = 8.0;

// Allocate columns only when both fit. Child UIs grow to the actual wrapped
// content height, and controls can wrap without painting into the text column.
pub(super) fn responsive_settings_row(
    ui: &mut egui::Ui,
    preferred_control_width: f32,
    add_text: impl FnOnce(&mut egui::Ui),
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let total_width = ui.available_width().max(1.0);
    let gap = ui.spacing().item_spacing.x.max(SETTINGS_COLUMN_GAP);
    let control_width = preferred_control_width.min(total_width);
    let remaining_text_width = total_width - control_width - gap;
    // Every setting shares a trailing control column; extra width is available
    // to descriptions instead of becoming empty space after the controls.
    let text_width = remaining_text_width;
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
            ui.add_space(2.0);
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
            add_text_column(ui);
            add_control_column(ui);
        });
    }
}

pub(super) fn bounded_control_width(ui: &egui::Ui, preferred: f32) -> f32 {
    preferred.min(ui.available_width().max(1.0))
}

// Two panes are used only when each can keep readable labels and controls.
// Independent IDs prevent repeated controls from sharing egui widget state.
pub(super) fn responsive_settings_columns<T>(
    ui: &mut egui::Ui,
    id: &'static str,
    state: &mut T,
    first: impl FnOnce(&mut egui::Ui, &mut T),
    second: impl FnOnce(&mut egui::Ui, &mut T),
) {
    if ui.available_width() >= 900.0 {
        ui.columns(2, |columns| {
            columns[0].push_id((id, 0), |ui| first(ui, state));
            columns[1].push_id((id, 1), |ui| second(ui, state));
        });
    } else {
        ui.push_id((id, 0), |ui| first(ui, state));
        ui.push_id((id, 1), |ui| second(ui, state));
    }
}
