#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../fonts.rs"]
mod fonts;

use eframe::egui::{
    self, Color32, FontId, Frame, Margin, RichText, Rounding, ScrollArea, Sense,
    Stroke as EguiStroke,
};
use pinyin_ime::handwrite_lookup::{match_typed, Match as LookupMatch, Point, Stroke};
use pinyin_ime::win_paste;

const WINDOW_TITLE: &str = "开心输入法 手写查字";
const DEFAULT_WINDOW_SIZE: [f32; 2] = [720.0, 520.0];
const MIN_WINDOW_SIZE: [f32; 2] = [560.0, 420.0];
const MAX_CANDIDATES: usize = 18;
const MIN_POINT_DISTANCE: f32 = 2.0;

struct HandwriteApp {
    strokes: Vec<Vec<Point>>,
    candidates: Vec<LookupMatch>,
    selected: usize,
    status: String,
    target_hwnd: isize,
    quick_paste: bool,
}

impl HandwriteApp {
    fn new(cc: &eframe::CreationContext<'_>, target_hwnd: isize) -> Self {
        let _ = fonts::install_cjk_fonts(&cc.egui_ctx);
        Self {
            strokes: Vec::new(),
            candidates: Vec::new(),
            selected: 0,
            status: "就绪".to_string(),
            target_hwnd,
            quick_paste: true,
        }
    }

    fn lookup(&mut self) {
        let strokes = self.lookup_strokes();
        if strokes.is_empty() {
            self.candidates.clear();
            self.selected = 0;
            self.status = "就绪".to_string();
            return;
        }
        self.candidates = match_typed(&strokes, MAX_CANDIDATES);
        self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
        self.status = format!("{} 笔，{} 个候选", strokes.len(), self.candidates.len());
    }

    fn lookup_strokes(&self) -> Vec<Stroke> {
        self.strokes
            .iter()
            .filter(|stroke| stroke.len() >= 2)
            .map(|points| Stroke {
                points: points.clone(),
            })
            .collect()
    }

    fn clear(&mut self) {
        self.strokes.clear();
        self.lookup();
    }

    fn undo_stroke(&mut self) {
        self.strokes.pop();
        self.lookup();
    }

    fn selected_text(&self) -> Option<String> {
        self.candidates
            .get(self.selected)
            .map(|candidate| candidate.hanzi.to_string())
    }

    fn copy_selected(&mut self) {
        let Some(text) = self.selected_text() else {
            self.status = "没有可复制的候选".to_string();
            return;
        };
        match copy_to_system_clipboard(&text) {
            Ok(()) => self.status = format!("已复制 {text}"),
            Err(err) => self.status = format!("复制失败：{err}"),
        }
    }

    fn paste_selected(&mut self, ctx: &egui::Context) {
        let Some(text) = self.selected_text() else {
            self.status = "没有可粘贴的候选。".to_string();
            return;
        };
        match copy_to_system_clipboard(&text) {
            Ok(()) => match win_paste::send_ctrl_v_to_target(self.target_hwnd) {
                Ok(()) => {
                    self.status = format!("已粘贴 {text}");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(err) => self.status = format!("已复制，但快粘失败：{err}"),
            },
            Err(err) => self.status = format!("粘贴失败：{err}"),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let mut paste = false;
        let mut copy = false;
        let mut clear = false;
        let mut undo = false;
        let mut next_selection: Option<usize> = None;

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                clear = true;
            }
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::Z) {
                undo = true;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace) {
                undo = true;
            }
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::C) {
                copy = true;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                paste = true;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                next_selection =
                    Some((self.selected + 1).min(self.candidates.len().saturating_sub(1)));
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                next_selection = Some(self.selected.saturating_sub(1));
            }
            let keys = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ];
            for (idx, key) in keys.iter().enumerate() {
                if input.consume_key(egui::Modifiers::NONE, *key)
                    && top_row_digit_pressed((idx + 1) as u8)
                    && idx < self.candidates.len()
                {
                    next_selection = Some(idx);
                    if self.quick_paste {
                        paste = true;
                    }
                    break;
                }
            }
        });

        if let Some(idx) = next_selection {
            self.selected = idx;
        }
        if undo {
            self.undo_stroke();
        }
        if clear {
            self.clear();
        }
        if copy {
            self.copy_selected();
        }
        if paste {
            self.paste_selected(ctx);
        }
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui) {
        let side = ui
            .available_width()
            .min(ui.available_height() - 58.0)
            .clamp(260.0, 380.0);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), Sense::drag());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, Rounding::same(8.0), Color32::WHITE);
        painter.rect_stroke(
            rect,
            Rounding::same(8.0),
            EguiStroke::new(1.0, Color32::from_rgb(203, 213, 225)),
        );

        let grid = Color32::from_rgb(226, 232, 240);
        painter.line_segment(
            [rect.left_center(), rect.right_center()],
            EguiStroke::new(1.0, grid),
        );
        painter.line_segment(
            [rect.center_top(), rect.center_bottom()],
            EguiStroke::new(1.0, grid),
        );

        if response.drag_started_by(egui::PointerButton::Primary) {
            self.strokes.push(Vec::new());
            if let Some(pos) = response.interact_pointer_pos() {
                self.push_canvas_point(rect, pos);
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                self.push_canvas_point(rect, pos);
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if self.strokes.last().is_some_and(|stroke| stroke.len() < 2) {
                self.strokes.pop();
            }
            self.lookup();
        }

        for stroke in &self.strokes {
            let points: Vec<_> = stroke
                .iter()
                .map(|point| canvas_pos(rect, *point))
                .collect();
            for pair in points.windows(2) {
                painter.line_segment(
                    [pair[0], pair[1]],
                    EguiStroke::new(5.0, Color32::from_rgb(15, 23, 42)),
                );
            }
            for point in points {
                painter.circle_filled(point, 2.3, Color32::from_rgb(15, 23, 42));
            }
        }

        if self.strokes.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "写",
                FontId::proportional(96.0),
                Color32::from_gray(220),
            );
        }
    }

    fn push_canvas_point(&mut self, rect: egui::Rect, pos: egui::Pos2) {
        let point = normalize_canvas_point(rect, pos);
        let Some(stroke) = self.strokes.last_mut() else {
            return;
        };
        if stroke
            .last()
            .is_some_and(|last| point_distance(*last, point) < MIN_POINT_DISTANCE)
        {
            return;
        }
        stroke.push(point);
    }

    fn show_candidate_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading("候选");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.quick_paste, "快粘");
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("复制").clicked() {
                self.copy_selected();
            }
            if ui.button("粘贴").clicked() {
                self.paste_selected(ctx);
            }
        });
        ui.add_space(8.0);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut paste_idx = None;
                for (idx, candidate) in self.candidates.iter().enumerate() {
                    let response = candidate_row(ui, idx, candidate, idx == self.selected);
                    if response.clicked() {
                        self.selected = idx;
                    }
                    if response.double_clicked() {
                        paste_idx = Some(idx);
                    }
                }
                if let Some(idx) = paste_idx {
                    self.selected = idx;
                    self.paste_selected(ctx);
                }
                if self.candidates.is_empty() {
                    ui.add_space(60.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("暂无候选").weak());
                    });
                }
            });
    }
}

impl eframe::App for HandwriteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);

        egui::TopBottomPanel::top("handwrite_top")
            .exact_height(54.0)
            .frame(Frame::none().fill(Color32::from_rgb(248, 250, 252)))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("手写查字");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("清空").clicked() {
                            self.clear();
                        }
                        if ui.button("撤销").clicked() {
                            self.undo_stroke();
                        }
                    });
                });
            });

        egui::TopBottomPanel::bottom("handwrite_status")
            .exact_height(36.0)
            .frame(Frame::none().fill(Color32::from_rgb(248, 250, 252)))
            .show(ctx, |ui| {
                ui.add_space(5.0);
                ui.label(RichText::new(&self.status).small().weak());
            });

        egui::CentralPanel::default()
            .frame(Frame::none().fill(Color32::from_rgb(248, 250, 252)))
            .show(ctx, |ui| {
                ui.columns(2, |columns| {
                    columns[0].vertical_centered(|ui| {
                        self.show_canvas(ui);
                    });
                    columns[1].vertical(|ui| {
                        self.show_candidate_panel(ui, ctx);
                    });
                });
            });
    }
}

fn candidate_row(
    ui: &mut egui::Ui,
    idx: usize,
    candidate: &LookupMatch,
    selected: bool,
) -> egui::Response {
    let fill = if selected {
        Color32::from_rgb(239, 246, 255)
    } else {
        Color32::WHITE
    };
    let stroke = if selected {
        Color32::from_rgb(59, 130, 246)
    } else {
        Color32::from_rgb(226, 232, 240)
    };
    let response = Frame::none()
        .fill(fill)
        .stroke(EguiStroke::new(1.0, stroke))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}", idx + 1))
                        .strong()
                        .color(Color32::from_rgb(37, 99, 235)),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(candidate.hanzi.to_string())
                        .font(FontId::proportional(34.0))
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.2}", candidate.score))
                            .small()
                            .color(Color32::from_rgb(100, 116, 139)),
                    );
                });
            });
        })
        .response;
    ui.add_space(6.0);
    response
}

fn normalize_canvas_point(rect: egui::Rect, pos: egui::Pos2) -> Point {
    let x = ((pos.x - rect.left()) / rect.width() * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8;
    let y = ((pos.y - rect.top()) / rect.height() * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8;
    Point { x, y }
}

fn canvas_pos(rect: egui::Rect, point: Point) -> egui::Pos2 {
    egui::pos2(
        rect.left() + rect.width() * (point.x as f32 / 255.0),
        rect.top() + rect.height() * (point.y as f32 / 255.0),
    )
}

fn point_distance(a: Point, b: Point) -> f32 {
    let dx = a.x as f32 - b.x as f32;
    let dy = a.y as f32 - b.y as f32;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(windows)]
fn copy_to_system_clipboard(text: &str) -> Result<(), String> {
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text)
        .map_err(|e| format!("copy clipboard text: {e}"))
}

#[cfg(not(windows))]
fn copy_to_system_clipboard(_text: &str) -> Result<(), String> {
    Err("clipboard quick paste is Windows-only".to_string())
}

#[cfg(windows)]
fn top_row_digit_pressed(d: u8) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    let d = i32::from(d);
    let vk_main = 0x30 + d;
    let vk_numpad = 0x60 + d;
    unsafe {
        let n = GetAsyncKeyState(vk_numpad) as u16;
        let m = GetAsyncKeyState(vk_main) as u16;
        (m & 0x8000 != 0) && (n & 0x8000 == 0)
    }
}

#[cfg(not(windows))]
fn top_row_digit_pressed(_d: u8) -> bool {
    true
}

#[cfg(windows)]
fn current_foreground_hwnd() -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow() }
}

#[cfg(not(windows))]
fn current_foreground_hwnd() -> isize {
    0
}

#[cfg(windows)]
fn exit_if_second_instance_activate_first() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    let mutex_name: Vec<u16> = "Local\\KaixinInput_Handwrite_SingleInstance_v1"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if mutex == 0 {
        return false;
    }
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return false;
    }
    unsafe {
        CloseHandle(mutex);
    }
    let title: Vec<u16> = WINDOW_TITLE
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd != 0 {
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
    true
}

#[cfg(not(windows))]
fn exit_if_second_instance_activate_first() -> bool {
    false
}

fn main() -> eframe::Result<()> {
    pinyin_ime::windows_security::apply_process_hardening();
    if exit_if_second_instance_activate_first() {
        return Ok(());
    }
    let target_hwnd = current_foreground_hwnd();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(DEFAULT_WINDOW_SIZE)
            .with_min_inner_size(MIN_WINDOW_SIZE)
            .with_title(WINDOW_TITLE),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(HandwriteApp::new(cc, target_hwnd)))),
    )
}
