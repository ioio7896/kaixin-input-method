mod fonts;
#[cfg(windows)]
mod ime_win_input;

use eframe::egui::{self, FontId, RichText, TextStyle};
use pinyin_ime::core::{
    CandidateMeta as EngineCandidateMeta, CandidateSource as EngineCandidateSource, PinyinEngine,
};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const PAGE_SIZE: usize = 9;
const GRID_COLUMNS: usize = 3;
const INPUT_DEBOUNCE: Duration = Duration::from_millis(35);
const ACTION_STATUS_VISIBLE: Duration = Duration::from_secs(2);

#[inline]
fn pinyin_edit_id() -> egui::Id {
    egui::Id::new("pinyin_main")
}

#[derive(Clone, Debug)]
struct Candidate {
    phrase: String,
    score: f64,
    meta: EngineCandidateMeta,
}

struct CandidateMenuData {
    phrase: String,
    is_user: bool,
    is_pinned: bool,
    is_blocked: bool,
    corrected_reading: Option<String>,
    raw_meta: Option<String>,
}

fn engine_meta_display_label(meta: &EngineCandidateMeta) -> Option<&str> {
    let label = meta
        .display_text()?
        .split('\t')
        .next()
        .unwrap_or_default()
        .trim();
    if label.starts_with("Emoji:") || label.starts_with("符号:") {
        Some(label)
    } else {
        None
    }
}

fn engine_meta_visible_label(meta: &EngineCandidateMeta) -> Option<&str> {
    if let Some(label) = engine_meta_display_label(meta) {
        return Some(label);
    }
    if meta.partial {
        return Some("加载中");
    }
    match meta.source {
        EngineCandidateSource::System => None,
        EngineCandidateSource::Direct => Some("直输"),
        EngineCandidateSource::Shortcut => Some("短语"),
        EngineCandidateSource::User => Some("用户词"),
        EngineCandidateSource::Correction => Some("纠错"),
        EngineCandidateSource::Abbrev => Some("简拼"),
        EngineCandidateSource::Mixed => Some("混拼"),
        EngineCandidateSource::English => Some("英文"),
        EngineCandidateSource::Pinyin => Some("全拼"),
        EngineCandidateSource::UMode => Some("拆字"),
        EngineCandidateSource::DateTime => Some("实时"),
    }
}

fn engine_meta_is_user(meta: &EngineCandidateMeta) -> bool {
    matches!(
        meta.source,
        EngineCandidateSource::User | EngineCandidateSource::Shortcut
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateBadge {
    Pinned,
    Blocked,
    Learned,
    Removed,
}

impl CandidateBadge {
    fn for_action(kind: CandidateActionKind) -> Option<Self> {
        match kind {
            CandidateActionKind::Pin => Some(Self::Pinned),
            CandidateActionKind::Learn => Some(Self::Learned),
            CandidateActionKind::Remove => Some(Self::Removed),
            CandidateActionKind::Block => Some(Self::Blocked),
            CandidateActionKind::Unpin
            | CandidateActionKind::Unblock
            | CandidateActionKind::LearnCorrection => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pinned => "📌 已置顶",
            Self::Blocked => "🚫 已屏蔽",
            Self::Learned => "已学习",
            Self::Removed => "已删除",
        }
    }
}

struct ActionStatus {
    message: String,
    expires_at: Instant,
}

struct LookupResult {
    request_id: u64,
    input: String,
    syllable_predictions: Vec<String>,
    candidates: Vec<Candidate>,
    error: Option<String>,
}

struct ActionResult {
    refresh: bool,
    message: Option<String>,
    kind: CandidateActionKind,
    phrase: String,
}

#[derive(Clone, Copy, Debug)]
enum CandidateActionKind {
    Pin,
    Unpin,
    Learn,
    Remove,
    Block,
    Unblock,
    LearnCorrection,
}

enum EngineCommand {
    Lookup {
        request_id: u64,
        input: String,
    },
    LearnSelection {
        reading: String,
        phrase: String,
        selected_index: usize,
        page: usize,
        skipped_candidates: Vec<String>,
    },
    CandidateAction {
        kind: CandidateActionKind,
        reading: String,
        phrase: String,
        corrected_reading: Option<String>,
    },
}

struct EngineWorker {
    tx: Sender<EngineCommand>,
    rx: Receiver<LookupResult>,
    action_rx: Receiver<ActionResult>,
    char_count: usize,
}

impl EngineWorker {
    fn new() -> Self {
        let mut ui_engine = PinyinEngine::new();
        let char_count = ui_engine.char_count;
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>();
        let (result_tx, result_rx) = mpsc::channel::<LookupResult>();
        let (action_tx, action_rx) = mpsc::channel::<ActionResult>();

        thread::spawn(move || {
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    EngineCommand::Lookup { request_id, input } => {
                        let (ranked, syllable_predictions, error) =
                            ui_engine.lookup_with_syllable_predictions(&input, request_id);
                        let candidates = ranked
                            .into_iter()
                            .map(|(phrase, score, meta)| Candidate {
                                phrase,
                                score,
                                meta,
                            })
                            .collect();
                        let _ = result_tx.send(LookupResult {
                            request_id,
                            input,
                            syllable_predictions,
                            candidates,
                            error,
                        });
                    }
                    EngineCommand::LearnSelection {
                        reading,
                        phrase,
                        selected_index,
                        page,
                        skipped_candidates,
                    } => {
                        let _ = ui_engine.learn_selection_feedback(
                            &reading,
                            &phrase,
                            selected_index,
                            page,
                            &skipped_candidates,
                        );
                    }
                    EngineCommand::CandidateAction {
                        kind,
                        reading,
                        phrase,
                        corrected_reading,
                    } => {
                        let result = match kind {
                            CandidateActionKind::Pin => ui_engine
                                .set_candidate_pin(&reading, &phrase, true)
                                .map(|_| true),
                            CandidateActionKind::Unpin => ui_engine
                                .set_candidate_pin(&reading, &phrase, false)
                                .map(|_| true),
                            CandidateActionKind::Learn => {
                                ui_engine.learn_commit(&reading, &phrase).map(|_| true)
                            }
                            CandidateActionKind::Remove => {
                                ui_engine.remove_user_phrase(&reading, &phrase)
                            }
                            CandidateActionKind::Block => ui_engine.block_user_phrase(&phrase),
                            CandidateActionKind::Unblock => ui_engine.unblock_user_phrase(&phrase),
                            CandidateActionKind::LearnCorrection => corrected_reading
                                .as_deref()
                                .ok_or_else(|| "missing corrected reading".to_string())
                                .and_then(|corrected| {
                                    ui_engine
                                        .learn_correction(&reading, corrected)
                                        .map(|_| true)
                                }),
                        };
                        let _ = action_tx.send(ActionResult {
                            refresh: result.as_ref().map(|changed| *changed).unwrap_or(false),
                            message: result.err(),
                            kind,
                            phrase,
                        });
                    }
                }
            }
        });

        Self {
            tx: cmd_tx,
            rx: result_rx,
            action_rx,
            char_count,
        }
    }

    fn lookup(&self, request_id: u64, input: String) {
        let _ = self.tx.send(EngineCommand::Lookup { request_id, input });
    }

    fn learn_selection(
        &self,
        reading: String,
        phrase: String,
        selected_index: usize,
        page: usize,
        skipped_candidates: Vec<String>,
    ) {
        let _ = self.tx.send(EngineCommand::LearnSelection {
            reading,
            phrase,
            selected_index,
            page,
            skipped_candidates,
        });
    }

    fn candidate_action(
        &self,
        kind: CandidateActionKind,
        reading: String,
        phrase: String,
        corrected_reading: Option<String>,
    ) {
        let _ = self.tx.send(EngineCommand::CandidateAction {
            kind,
            reading,
            phrase,
            corrected_reading,
        });
    }
}

struct ImeApp {
    committed: String,
    pinyin_buffer: String,
    syllable_predictions: Vec<String>,
    candidates: Vec<Candidate>,
    candidate_page: usize,
    highlight_in_page: usize,
    error: Option<String>,
    worker: EngineWorker,
    parse_engine: PinyinEngine,
    cjk_font_hint: Option<String>,
    refocus_pinyin: bool,
    next_request_id: u64,
    latest_request_id: u64,
    is_loading_candidates: bool,
    popup_anchor: Option<egui::Pos2>,
    pending_lookup_due: Option<Instant>,
    debug_candidate_list: bool,
    action_status: Option<ActionStatus>,
    pending_candidate_badges: HashMap<String, CandidateBadge>,
}

struct ImeRoot {
    app: ImeApp,
}

impl eframe::App for ImeRoot {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(windows)]
        ime_win_input::filter_raw_input(raw_input, self.app.has_candidates());
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);
    }
}

impl ImeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cjk_font_hint = fonts::install_cjk_fonts(&cc.egui_ctx);
        let mut app = Self {
            committed: String::new(),
            pinyin_buffer: String::new(),
            syllable_predictions: Vec::new(),
            candidates: Vec::new(),
            candidate_page: 0,
            highlight_in_page: 0,
            error: None,
            worker: EngineWorker::new(),
            parse_engine: PinyinEngine::new(),
            cjk_font_hint,
            refocus_pinyin: true,
            next_request_id: 1,
            latest_request_id: 0,
            is_loading_candidates: false,
            popup_anchor: None,
            pending_lookup_due: None,
            debug_candidate_list: false,
            action_status: None,
            pending_candidate_badges: HashMap::new(),
        };
        app.refresh_candidates();
        app
    }

    fn refresh_candidates(&mut self) {
        self.pending_lookup_due = None;
        self.error = None;
        if self.pinyin_buffer.trim().is_empty() {
            self.clear_candidates();
            return;
        }

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.latest_request_id = request_id;
        self.candidate_page = 0;
        self.highlight_in_page = 0;
        self.is_loading_candidates = true;
        self.worker.lookup(request_id, self.pinyin_buffer.clone());
    }

    fn schedule_candidate_refresh(&mut self, ctx: &egui::Context) {
        self.error = None;
        if self.pinyin_buffer.trim().is_empty() {
            self.pending_lookup_due = None;
            self.clear_candidates();
            return;
        }
        self.candidate_page = 0;
        self.highlight_in_page = 0;
        self.is_loading_candidates = true;
        let due = Instant::now() + INPUT_DEBOUNCE;
        self.pending_lookup_due = Some(due);
        ctx.request_repaint_after(INPUT_DEBOUNCE);
    }

    fn flush_pending_lookup_if_due(&mut self, ctx: &egui::Context) {
        let Some(due) = self.pending_lookup_due else {
            return;
        };
        let now = Instant::now();
        if now >= due {
            self.refresh_candidates();
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(due.saturating_duration_since(now));
        }
    }

    fn clear_candidates(&mut self) {
        self.pending_lookup_due = None;
        self.candidates.clear();
        self.syllable_predictions.clear();
        self.candidate_page = 0;
        self.highlight_in_page = 0;
        self.is_loading_candidates = false;
        self.pending_candidate_badges.clear();
    }

    fn set_action_error(&mut self, message: String, ctx: &egui::Context) {
        self.action_status = Some(ActionStatus {
            message,
            expires_at: Instant::now() + ACTION_STATUS_VISIBLE,
        });
        ctx.request_repaint_after(ACTION_STATUS_VISIBLE);
    }

    fn clear_expired_action_status(&mut self, ctx: &egui::Context) {
        let Some(status) = self.action_status.as_ref() else {
            return;
        };
        let now = Instant::now();
        if now >= status.expires_at {
            self.action_status = None;
        } else {
            ctx.request_repaint_after(status.expires_at.saturating_duration_since(now));
        }
    }

    fn poll_lookup_results(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.worker.rx.try_recv() {
            if result.request_id != self.latest_request_id || result.input != self.pinyin_buffer {
                continue;
            }
            self.syllable_predictions = result.syllable_predictions;
            self.candidates = result.candidates;
            self.error = result.error;
            self.is_loading_candidates = false;
            self.clamp_page();
            ctx.request_repaint();
        }

        let mut should_refresh = false;
        while let Ok(result) = self.worker.action_rx.try_recv() {
            if let Some(message) = result.message {
                self.pending_candidate_badges.remove(&result.phrase);
                self.set_action_error(message, ctx);
            } else if matches!(
                result.kind,
                CandidateActionKind::Unpin
                    | CandidateActionKind::Unblock
                    | CandidateActionKind::LearnCorrection
            ) || !result.refresh
            {
                self.pending_candidate_badges.remove(&result.phrase);
            }
            should_refresh |= result.refresh;
        }
        if should_refresh {
            self.refresh_candidates();
            self.refocus_pinyin = true;
            ctx.request_repaint();
        }
    }

    fn max_page(&self) -> usize {
        if self.candidates.is_empty() {
            0
        } else {
            (self.candidates.len() - 1) / PAGE_SIZE
        }
    }

    fn current_page_count(&self) -> usize {
        self.candidates
            .len()
            .saturating_sub(self.candidate_page * PAGE_SIZE)
            .min(PAGE_SIZE)
    }

    fn clamp_page(&mut self) {
        let m = self.max_page();
        if self.candidate_page > m {
            self.candidate_page = m;
            self.highlight_in_page = 0;
        }
        let page_count = self.current_page_count();
        if page_count == 0 {
            self.highlight_in_page = 0;
        } else {
            self.highlight_in_page = self.highlight_in_page.min(page_count - 1);
        }
    }

    fn next_page(&mut self) {
        let old = self.candidate_page;
        self.candidate_page = (self.candidate_page + 1).min(self.max_page());
        if self.candidate_page != old {
            self.highlight_in_page = 0;
        }
    }

    fn previous_page(&mut self) {
        let old = self.candidate_page;
        self.candidate_page = self.candidate_page.saturating_sub(1);
        if self.candidate_page != old {
            self.highlight_in_page = 0;
        }
    }

    fn commit_candidate(&mut self, idx: usize) {
        if idx >= self.candidates.len() {
            return;
        }

        let reading = self.pinyin_buffer.clone();
        let candidate = self.candidates[idx].clone();
        let skipped_candidates = self
            .candidates
            .iter()
            .take(idx)
            .map(|candidate| candidate.phrase.clone())
            .collect::<Vec<_>>();
        self.worker.learn_selection(
            reading,
            candidate.phrase.clone(),
            idx,
            idx / PAGE_SIZE,
            skipped_candidates,
        );

        let single = candidate
            .phrase
            .chars()
            .filter(|c| !c.is_whitespace())
            .count()
            == 1;
        if single {
            if let Some(rest) = self
                .parse_engine
                .strip_leading_syllable_from_reading(&self.pinyin_buffer)
            {
                self.committed.push_str(&candidate.phrase);
                self.pinyin_buffer = rest;
                self.refresh_candidates();
                self.refocus_pinyin = true;
                return;
            }
        }

        self.committed.push_str(&candidate.phrase);
        self.pinyin_buffer.clear();
        self.clear_candidates();
        self.error = None;
        self.refocus_pinyin = true;
    }

    fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    fn handle_ime_keys(&mut self, ctx: &egui::Context) {
        if self.candidates.is_empty() {
            return;
        }

        let mut commit_idx: Option<usize> = None;

        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::PageDown)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Equals)
            {
                self.next_page();
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::PageUp)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Minus)
            {
                self.previous_page();
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
            for (k, key) in keys.iter().enumerate() {
                if i.consume_key(egui::Modifiers::NONE, *key) {
                    let g = self.candidate_page * PAGE_SIZE + k;
                    if g < self.candidates.len() {
                        commit_idx = Some(g);
                    }
                    return;
                }
            }

            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                let g = self.candidate_page * PAGE_SIZE + self.highlight_in_page;
                if g < self.candidates.len() {
                    commit_idx = Some(g);
                }
            }
        });

        if let Some(idx) = commit_idx {
            self.commit_candidate(idx);
            return;
        }

        ctx.input_mut(|i| {
            let page_count = self.current_page_count();
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) && page_count > 0 {
                self.highlight_in_page = (self.highlight_in_page + 1).min(page_count - 1);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                self.highlight_in_page = self.highlight_in_page.saturating_sub(1);
            }
        });
    }

    fn widget_rect_to_screen(
        &self,
        ctx: &egui::Context,
        layer_id: egui::LayerId,
        rect: egui::Rect,
    ) -> egui::Rect {
        ctx.memory(|m| {
            m.layer_transforms
                .get(&layer_id)
                .map(|t| *t * rect)
                .unwrap_or(rect)
        })
    }

    fn candidate_font_size(ui: &egui::Ui) -> f32 {
        let body = ui.text_style_height(&TextStyle::Body);
        let ppp = ui.ctx().pixels_per_point();
        (body + 3.0 * ppp).clamp(18.0, 22.0)
    }

    fn badge_for_candidate(&self, candidate: &Candidate) -> Option<CandidateBadge> {
        self.pending_candidate_badges
            .get(&candidate.phrase)
            .copied()
            .or_else(|| candidate.meta.pinned.then_some(CandidateBadge::Pinned))
            .or_else(|| candidate.meta.blocked.then_some(CandidateBadge::Blocked))
    }

    fn draw_candidate_cell(
        ui: &mut egui::Ui,
        number: usize,
        candidate: &Candidate,
        active: bool,
        badge: Option<CandidateBadge>,
        width: f32,
        font_size: f32,
    ) -> egui::Response {
        let accent = egui::Color32::from_rgb(30, 115, 210);
        let active_fill = egui::Color32::from_rgb(225, 239, 255);
        let stroke = if active {
            egui::Stroke::new(1.2, accent)
        } else {
            egui::Stroke::new(1.0, egui::Color32::from_gray(190))
        };
        let fill = if active {
            active_fill
        } else {
            egui::Color32::TRANSPARENT
        };

        let response = egui::Frame::none()
            .fill(fill)
            .stroke(stroke)
            .rounding(egui::Rounding::same(6.0))
            .inner_margin(egui::Margin::same(7.0))
            .show(ui, |ui| {
                ui.set_min_size(egui::vec2(width, 52.0));
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{}.", number))
                                .color(if active {
                                    accent
                                } else {
                                    egui::Color32::from_gray(110)
                                })
                                .strong(),
                        );
                        ui.label(
                            RichText::new(&candidate.phrase)
                                .font(FontId::proportional(font_size))
                                .color(if active {
                                    egui::Color32::from_rgb(12, 66, 130)
                                } else {
                                    ui.visuals().text_color()
                                })
                                .strong(),
                        );
                        if let Some(badge) = badge {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(badge.label())
                                            .small()
                                            .color(egui::Color32::from_rgb(180, 78, 42)),
                                    );
                                },
                            );
                        }
                    });
                    if let Some(meta) = engine_meta_visible_label(&candidate.meta) {
                        ui.label(RichText::new(meta).small().color(if active {
                            accent
                        } else {
                            egui::Color32::from_gray(120)
                        }));
                    }
                });
            })
            .response
            .interact(egui::Sense::click());

        if let Some(meta) = candidate.meta.display_text() {
            response.on_hover_text(format!("{}\nscore: {:.2}", meta, candidate.score))
        } else {
            response.on_hover_text(format!("score: {:.2}", candidate.score))
        }
    }

    fn show_candidate_grid(
        &mut self,
        ui: &mut egui::Ui,
        grid_id: egui::Id,
        cell_width: f32,
        font_size: f32,
    ) -> Option<usize> {
        let start = self.candidate_page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(self.candidates.len());
        let mut clicked = None;

        // Use a stable grid id so egui can persist hover/highlight/context-menu
        // state across frames. ui.next_auto_id() changes every frame and causes
        // flicker when candidates update, and breaks context-menu anchoring.
        egui::Grid::new(grid_id)
            .num_columns(GRID_COLUMNS)
            .spacing(egui::vec2(8.0, 8.0))
            .show(ui, |ui| {
                for local in 0..PAGE_SIZE {
                    if local > 0 && local % GRID_COLUMNS == 0 {
                        ui.end_row();
                    }
                    let global = start + local;
                    if global < end {
                        let badge = self.badge_for_candidate(&self.candidates[global]);
                        let response = Self::draw_candidate_cell(
                            ui,
                            local + 1,
                            &self.candidates[global],
                            local == self.highlight_in_page,
                            badge,
                            cell_width,
                            font_size,
                        );
                        if response.hovered() {
                            if self.highlight_in_page != local {
                                self.highlight_in_page = local;
                                ui.ctx().request_repaint();
                            }
                        }
                        let candidate = &self.candidates[global];
                        let menu_data = CandidateMenuData {
                            phrase: candidate.phrase.clone(),
                            is_user: engine_meta_is_user(&candidate.meta),
                            is_pinned: candidate.meta.pinned,
                            is_blocked: candidate.meta.blocked,
                            corrected_reading: candidate.meta.correction_target.clone(),
                            raw_meta: candidate.meta.display_text().map(str::to_string),
                        };
                        let reading = self.pinyin_buffer.clone();
                        self.show_candidate_context_menu(response.clone(), reading, menu_data);
                        if response.clicked() {
                            clicked = Some(global);
                        }
                    } else {
                        ui.allocate_space(egui::vec2(cell_width, 52.0));
                    }
                }
            });

        clicked
    }

    fn send_candidate_action(
        &mut self,
        kind: CandidateActionKind,
        reading: &str,
        phrase: &str,
        corrected_reading: Option<String>,
    ) {
        self.action_status = None;
        if let Some(badge) = CandidateBadge::for_action(kind) {
            self.pending_candidate_badges
                .insert(phrase.to_string(), badge);
        } else {
            self.pending_candidate_badges.remove(phrase);
        }
        self.worker.candidate_action(
            kind,
            reading.to_string(),
            phrase.to_string(),
            corrected_reading,
        );
        self.refocus_pinyin = true;
    }

    fn show_candidate_context_menu(
        &mut self,
        response: egui::Response,
        reading: String,
        candidate: CandidateMenuData,
    ) {
        response.context_menu(|ui| {
            if ui
                .add_enabled(!candidate.is_pinned, egui::Button::new("置顶此候选"))
                .clicked()
            {
                self.send_candidate_action(
                    CandidateActionKind::Pin,
                    &reading,
                    &candidate.phrase,
                    None,
                );
                ui.close_menu();
            }
            if ui
                .add_enabled(candidate.is_pinned, egui::Button::new("取消置顶"))
                .clicked()
            {
                self.send_candidate_action(
                    CandidateActionKind::Unpin,
                    &reading,
                    &candidate.phrase,
                    None,
                );
                ui.close_menu();
            }
            ui.separator();
            if ui.button("加入用户词").clicked() {
                self.send_candidate_action(
                    CandidateActionKind::Learn,
                    &reading,
                    &candidate.phrase,
                    None,
                );
                ui.close_menu();
            }
            if ui
                .add_enabled(candidate.is_user, egui::Button::new("删除用户词"))
                .clicked()
            {
                self.send_candidate_action(
                    CandidateActionKind::Remove,
                    &reading,
                    &candidate.phrase,
                    None,
                );
                ui.close_menu();
            }
            if ui
                .add_enabled(!candidate.is_blocked, egui::Button::new("不再学习 / 屏蔽"))
                .clicked()
            {
                self.send_candidate_action(
                    CandidateActionKind::Block,
                    &reading,
                    &candidate.phrase,
                    None,
                );
                ui.close_menu();
            }
            if ui
                .add_enabled(candidate.is_blocked, egui::Button::new("恢复学习"))
                .clicked()
            {
                self.send_candidate_action(
                    CandidateActionKind::Unblock,
                    &reading,
                    &candidate.phrase,
                    None,
                );
                ui.close_menu();
            }
            if ui
                .add_enabled(
                    candidate.corrected_reading.is_some(),
                    egui::Button::new("记住纠错"),
                )
                .clicked()
            {
                self.send_candidate_action(
                    CandidateActionKind::LearnCorrection,
                    &reading,
                    &candidate.phrase,
                    candidate.corrected_reading.clone(),
                );
                ui.close_menu();
            }
            ui.separator();
            if ui.button("复制候选").clicked() {
                ui.ctx().copy_text(candidate.phrase.clone());
                self.refocus_pinyin = true;
                ui.close_menu();
            }
            if let Some(meta) = candidate.raw_meta.as_deref() {
                ui.label(RichText::new(format!("来源：{}", meta)).small().weak());
            } else {
                ui.label(RichText::new("来源：系统词").small().weak());
            }
        });
    }

    fn show_candidate_popup(&mut self, ctx: &egui::Context) {
        if self.candidates.is_empty() && !self.is_loading_candidates {
            return;
        }
        let Some(anchor) = self.popup_anchor else {
            return;
        };

        let estimated_height = 250.0;
        let screen_rect = ctx.screen_rect();
        let below = anchor + egui::vec2(0.0, 6.0);
        let above = anchor - egui::vec2(0.0, estimated_height + 34.0);
        let pos = if below.y + estimated_height <= screen_rect.bottom()
            || below.y < screen_rect.center().y
        {
            below
        } else {
            egui::pos2(anchor.x, above.y.max(screen_rect.top() + 8.0))
        };

        let mut clicked = None;
        egui::Area::new(egui::Id::new("ime_candidate_popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(150)))
                    .show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "候选 {} / {}",
                                    self.candidate_page + 1,
                                    self.max_page() + 1
                                ))
                                .weak()
                                .small(),
                            );
                            if self.is_loading_candidates {
                                ui.label(RichText::new("正在加载...").small().weak());
                            }
                        });
                        ui.separator();
                        let font_size = Self::candidate_font_size(ui);
                        clicked = self.show_candidate_grid(
                            ui,
                            egui::Id::new("candidate_grid_popup"),
                            128.0,
                            font_size,
                        );
                    });
            });

        if let Some(idx) = clicked {
            self.commit_candidate(idx);
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.flush_pending_lookup_if_due(ctx);
        self.clear_expired_action_status(ctx);
        self.poll_lookup_results(ctx);
        self.handle_ime_keys(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(RichText::new("本地拼音整句输入").size(20.0));
            ui.separator();
            if let Some(ref name) = self.cjk_font_hint {
                ui.label(format!("界面字体：{}", name));
            } else {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "未找到首选 CJK 字体，汉字可能显示异常。",
                );
            }
            ui.label(format!(
                "字库：约 {} 个简体汉字；主键盘 1-9 选词，[/] 或 -= 翻页，Enter 提交高亮候选。",
                self.worker.char_count
            ));

            ui.add_space(6.0);
            ui.label(RichText::new("已上屏文本").weak());
            ui.add(
                egui::TextEdit::multiline(&mut self.committed)
                    .desired_width(f32::INFINITY)
                    .desired_rows(5),
            );

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("组合区").weak());
                ui.label(
                    RichText::new(&self.pinyin_buffer)
                        .color(egui::Color32::from_rgb(0, 90, 160))
                        .monospace(),
                );
            });

            ui.horizontal(|ui| {
                ui.label("拼音输入：");
                let te = egui::TextEdit::singleline(&mut self.pinyin_buffer)
                    .id(pinyin_edit_id())
                    .desired_width(ui.available_width().min(520.0))
                    .hint_text("如 h / hao / wo ai bei jing");
                let response = ui.add(te);
                if self.refocus_pinyin {
                    response.request_focus();
                    self.refocus_pinyin = false;
                }
                if response.changed() {
                    self.schedule_candidate_refresh(ctx);
                }

                let screen_rect = self.widget_rect_to_screen(ctx, response.layer_id, response.rect);
                self.popup_anchor = Some(screen_rect.left_bottom());
            });

            if !self.debug_candidate_list {
                self.show_candidate_popup(ctx);
            }

            if let Some(first) = self.candidates.first() {
                if !self.pinyin_buffer.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("首选预览：").weak());
                        ui.label(
                            RichText::new(&first.phrase)
                                .font(FontId::proportional(Self::candidate_font_size(ui))),
                        );
                        if self.is_loading_candidates {
                            ui.label(RichText::new("更新中...").weak().small());
                        }
                    });
                }
            } else if self.is_loading_candidates {
                ui.label(RichText::new("候选正在加载...").weak());
            }

            if !self.syllable_predictions.is_empty() {
                let sample = self
                    .syllable_predictions
                    .iter()
                    .take(12)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                ui.label(
                    RichText::new(format!("音节补全预测：{}", sample))
                        .weak()
                        .small(),
                );
            }

            ui.horizontal(|ui| {
                if ui.button("刷新候选").clicked() {
                    self.refresh_candidates();
                }
                if ui.button("示例：wo ai bei jing").clicked() {
                    self.pinyin_buffer = "wo ai bei jing".into();
                    self.refresh_candidates();
                }
                if ui.button("清空上屏").clicked() {
                    self.committed.clear();
                }
                ui.checkbox(&mut self.debug_candidate_list, "Debug list");
            });

            ui.separator();
            if let Some(ref e) = self.error {
                ui.colored_label(egui::Color32::RED, e);
            }
            if let Some(ref status) = self.action_status {
                ui.colored_label(egui::Color32::YELLOW, &status.message);
            }

            if self.debug_candidate_list {
                ui.heading("候选列表");
                let font_size = Self::candidate_font_size(ui);
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            if let Some(idx) = self.show_candidate_grid(
                                ui,
                                egui::Id::new("candidate_grid_list"),
                                170.0,
                                font_size,
                            ) {
                                self.commit_candidate(idx);
                            }
                        });
                    });
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    pinyin_ime::windows_security::apply_process_hardening();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 640.0])
            .with_title("本地拼音输入法"),
        ..Default::default()
    };
    eframe::run_native(
        "pinyin-ime",
        options,
        Box::new(|cc| {
            Ok(Box::new(ImeRoot {
                app: ImeApp::new(cc),
            }))
        }),
    )
}
