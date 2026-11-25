use super::fonts;
#[path = "../srf_ime_keyboard/hangul.rs"]
mod hangul;
#[path = "../srf_ime_keyboard/kana.rs"]
mod kana;
#[path = "../srf_ime_keyboard/layouts.rs"]
mod layouts;
#[path = "../srf_ime_keyboard/no_activate.rs"]
mod no_activate;

use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Frame, Margin, Pos2, RichText, Rounding, Sense,
    Stroke, Vec2,
};
use hangul::HangulComposer;
use kana::{KanaComposer, KanaTransform};
use layouts::{CharKey, KeyTone, Language};
use pinyin_ime::{ui_theme, win_paste, win_single_instance, win_target};
use std::time::Duration;

const WINDOW_TITLE: &str = "开心输入法 屏幕键盘";
const DEFAULT_SIZE: [f32; 2] = [820.0, 344.0];
const MIN_SIZE: [f32; 2] = [690.0, 304.0];

#[derive(Debug, Clone, Copy)]
enum ControlKey {
    Backspace,
    Space,
    Enter,
    Left,
    Right,
}

struct KeyboardApp {
    language: Language,
    shift_once: bool,
    caps_lock: bool,
    katakana: bool,
    show_hints: bool,
    copy_only: bool,
    copy_buffer: String,
    hangul: HangulComposer,
    kana: KanaComposer,
    status: String,
    status_error: bool,
    _target_tracker: win_target::Tracker,
}

impl KeyboardApp {
    fn new(cc: &eframe::CreationContext<'_>, tracker: win_target::Tracker, lang: Language) -> Self {
        fonts::install_cjk_fonts(&cc.egui_ctx);
        let _ = ui_theme::sync_theme_from_prefs(&cc.egui_ctx);
        ui_theme::apply_tool_style(&cc.egui_ctx);
        Self {
            language: lang,
            shift_once: false,
            caps_lock: false,
            katakana: false,
            show_hints: true,
            copy_only: false,
            copy_buffer: String::new(),
            hangul: HangulComposer::new(),
            kana: KanaComposer::new(),
            status: "点击按键即可向当前编辑框连续输入，窗口置顶且不抢占焦点。".into(),
            status_error: false,
            _target_tracker: tracker,
        }
    }

    fn key_font_family(&self) -> FontFamily {
        let name = match self.language {
            Language::Russian => fonts::KEYBOARD_RU_FONT_FAMILY,
            Language::Japanese => fonts::KEYBOARD_JA_FONT_FAMILY,
            Language::Korean => fonts::KEYBOARD_KO_FONT_FAMILY,
        };
        FontFamily::Name(name.into())
    }

    fn active_preview(&self) -> Option<char> {
        match self.language {
            Language::Russian => None,
            Language::Japanese => self.kana.preview(),
            Language::Korean => self.hangul.preview(),
        }
    }

    fn switch_language(&mut self, next: Language) {
        if self.language == next {
            return;
        }
        self.flush_pending();
        self.language = next;
        self.shift_once = false;
        self.caps_lock = false;
        self.status = format!("已切换到 {}（{}）", next.label(), next.subtitle());
        self.status_error = false;
    }

    fn flush_pending(&mut self) {
        let pending = match self.language {
            Language::Russian => None,
            Language::Japanese => self.kana.flush(),
            Language::Korean => self.hangul.flush(),
        };
        if let Some(text) = pending {
            self.emit_text(&text);
        }
    }

    fn emit_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.copy_only {
            self.copy_buffer.push_str(text);
            self.status = format!("已写入缓冲：{text}");
            self.status_error = false;
            return;
        }
        let result = win_target::validated_target()
            .and_then(|hwnd| win_paste::send_unicode_text_to_target(hwnd, text));
        match result {
            Ok(()) => {
                self.status = format!("已输入 {text}");
                self.status_error = false;
            }
            Err(error) => {
                self.status = error;
                self.status_error = true;
            }
        }
    }

    fn handle_char_key(&mut self, key: &CharKey) {
        let shifted = match self.language {
            Language::Russian => self.shift_once || self.caps_lock,
            Language::Korean => self.shift_once,
            Language::Japanese => false,
        };
        let text = key.active_text(shifted);
        if self.shift_once {
            self.shift_once = false;
        }
        match self.language {
            Language::Russian => self.emit_text(text),
            Language::Korean => {
                let Some(ch) = text.chars().next() else {
                    return;
                };
                if key.tone == KeyTone::Consonant || key.tone == KeyTone::Vowel {
                    if let Some(committed) = self.hangul.push_jamo(ch) {
                        self.emit_text(&committed);
                    } else if let Some(preview) = self.hangul.preview() {
                        self.status = format!("正在组字：{preview}");
                        self.status_error = false;
                    }
                } else {
                    self.flush_pending();
                    self.emit_text(text);
                }
            }
            Language::Japanese => {
                self.flush_pending();
                self.emit_text(text);
            }
        }
    }

    fn handle_kana_key(&mut self, ch: char) {
        if let Some(committed) = self.kana.push_kana(ch) {
            self.emit_text(&committed);
        }
        if let Some(preview) = self.kana.preview() {
            self.status = format!("待变换假名：{preview}（可点 ゛ / ゜ / 小）");
            self.status_error = false;
        }
    }

    fn handle_kana_transform(&mut self, transform: KanaTransform) {
        if self.kana.apply(transform) {
            if let Some(preview) = self.kana.preview() {
                self.status = format!("已变换为：{preview}");
                self.status_error = false;
            }
        } else {
            self.status = "当前没有可变换的假名，请先点击可加浊音或小写的假名。".into();
            self.status_error = false;
        }
    }

    fn handle_direct_punctuation(&mut self, text: &str) {
        self.flush_pending();
        self.emit_text(text);
    }

    fn handle_control_key(&mut self, key: ControlKey) {
        match key {
            ControlKey::Backspace => {
                let consumed = match self.language {
                    Language::Russian => false,
                    Language::Japanese => self.kana.backspace(),
                    Language::Korean => self.hangul.backspace(),
                };
                if consumed {
                    self.status = match self.active_preview() {
                        Some(ch) => format!("退格后：{ch}"),
                        None => "已清除当前组字".into(),
                    };
                    self.status_error = false;
                    return;
                }
                if self.copy_only {
                    if self.copy_buffer.pop().is_some() {
                        self.status = "已删除缓冲末尾字符".into();
                    }
                    self.status_error = false;
                    return;
                }
                let result =
                    win_target::validated_target().and_then(win_paste::send_backspace_to_target);
                self.update_control_status(result, "已发送退格");
            }
            ControlKey::Space => {
                self.flush_pending();
                if self.copy_only {
                    self.copy_buffer.push(' ');
                    self.status = "已在缓冲加入空格".into();
                    self.status_error = false;
                    return;
                }
                let result =
                    win_target::validated_target().and_then(win_paste::send_space_to_target);
                self.update_control_status(result, "已发送空格");
            }
            ControlKey::Enter => {
                self.flush_pending();
                if self.copy_only {
                    self.copy_buffer.push('\n');
                    self.status = "已在缓冲换行".into();
                    self.status_error = false;
                    return;
                }
                let result =
                    win_target::validated_target().and_then(win_paste::send_enter_to_target);
                self.update_control_status(result, "已发送回车");
            }
            ControlKey::Left => {
                self.flush_pending();
                if self.copy_only {
                    return;
                }
                let result =
                    win_target::validated_target().and_then(win_paste::send_arrow_left_to_target);
                self.update_control_status(result, "光标左移");
            }
            ControlKey::Right => {
                self.flush_pending();
                if self.copy_only {
                    return;
                }
                let result =
                    win_target::validated_target().and_then(win_paste::send_arrow_right_to_target);
                self.update_control_status(result, "光标右移");
            }
        }
    }

    fn update_control_status(&mut self, result: Result<(), String>, ok_message: &str) {
        match result {
            Ok(()) => {
                self.status = ok_message.into();
                self.status_error = false;
            }
            Err(error) => {
                self.status = error;
                self.status_error = true;
            }
        }
    }

    fn copy_buffer_text(&mut self) {
        self.flush_pending();
        if self.copy_buffer.is_empty() {
            self.status = "缓冲内容为空，暂无可复制文本。".into();
            self.status_error = false;
            return;
        }
        match copy_text(&self.copy_buffer) {
            Ok(()) => {
                self.status = format!("已复制 {} 个字符到剪贴板", self.copy_buffer.chars().count());
                self.status_error = false;
            }
            Err(error) => {
                self.status = error;
                self.status_error = true;
            }
        }
    }

    fn show(&mut self, ctx: &egui::Context) {
        win_target::remember_foreground();
        ctx.request_repaint_after(Duration::from_millis(250));

        let palette = ui_theme::UiPalette::from_visuals(&ctx.style().visuals);

        egui::TopBottomPanel::top("keyboard_header")
            .frame(
                Frame::none()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(10.0, 6.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for lang in Language::ALL {
                        let selected = self.language == lang;
                        let button = egui::Button::new(
                            RichText::new(lang.label())
                                .size(13.0)
                                .strong()
                                .color(if selected {
                                    palette.accent_text
                                } else {
                                    palette.text
                                }),
                        )
                        .fill(if selected {
                            palette.accent
                        } else {
                            palette.control_bg
                        })
                        .rounding(Rounding::same(palette.control_radius));
                        if ui.add_sized([74.0, 26.0], button).clicked() {
                            self.switch_language(lang);
                        }
                    }

                    ui.add_space(6.0);
                    if let Some(preview) = self.active_preview() {
                        let tag = match self.language {
                            Language::Korean => "组字",
                            Language::Japanese => "假名",
                            Language::Russian => "",
                        };
                        let pill = egui::Button::new(
                            RichText::new(format!("{tag} {preview}"))
                                .font(FontId::new(15.0, self.key_font_family()))
                                .color(palette.accent),
                        )
                        .fill(palette.accent_soft)
                        .stroke(Stroke::new(1.0_f32, palette.accent))
                        .rounding(Rounding::same(palette.control_radius));
                        if ui
                            .add_sized([74.0, 26.0], pill)
                            .on_hover_text("点击立即定型上屏")
                            .clicked()
                        {
                            self.flush_pending();
                        }
                    } else {
                        ui.label(
                            RichText::new(self.language.subtitle())
                                .small()
                                .color(palette.muted),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let copy_changed = ui
                            .checkbox(&mut self.copy_only, "仅复制")
                            .on_hover_text("勾选后点击内容先暂存到缓冲栏，方便整句复制。")
                            .changed();
                        if copy_changed {
                            self.flush_pending();
                        }
                        ui.checkbox(&mut self.show_hints, "读音提示")
                            .on_hover_text("在键帽上显示罗马音或拉丁读音提示。");
                    });
                });

                if self.copy_only {
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        let mut preview_line = self.copy_buffer.clone();
                        if let Some(ch) = self.active_preview() {
                            preview_line.push('[');
                            preview_line.push(ch);
                            preview_line.push(']');
                        }
                        let display = if preview_line.is_empty() {
                            "缓冲栏（点击按键拼写后，点右侧复制）".to_owned()
                        } else {
                            preview_line.replace('\n', " ⏎ ")
                        };
                        let box_width = (ui.available_width() - 124.0).max(120.0);
                        Frame::none()
                            .fill(palette.app_bg)
                            .stroke(Stroke::new(1.0_f32, palette.border))
                            .rounding(Rounding::same(palette.control_radius))
                            .inner_margin(Margin::symmetric(8.0, 4.0))
                            .show(ui, |ui| {
                                ui.set_min_width(box_width);
                                ui.set_max_width(box_width);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(display)
                                            .font(FontId::new(14.0, self.key_font_family()))
                                            .color(
                                                if self.copy_buffer.is_empty()
                                                    && self.active_preview().is_none()
                                                {
                                                    palette.muted
                                                } else {
                                                    palette.text
                                                },
                                            ),
                                    )
                                    .truncate(),
                                );
                            });
                        if ui
                            .add_sized([54.0, 26.0], egui::Button::new("复制"))
                            .clicked()
                        {
                            self.copy_buffer_text();
                        }
                        if ui
                            .add_sized([54.0, 26.0], egui::Button::new("清空"))
                            .clicked()
                        {
                            self.copy_buffer.clear();
                            self.hangul.clear();
                            self.kana.clear();
                            self.status = "已清空缓冲栏".into();
                            self.status_error = false;
                        }
                    });
                }
            });

        egui::TopBottomPanel::bottom("keyboard_status")
            .frame(
                Frame::none()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(10.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&self.status).size(11.5).color(
                            if self.status_error {
                                palette.danger
                            } else {
                                palette.muted
                            },
                        ))
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let target_label = match win_target::target_window_title() {
                            Some(title) => format!("目标：{}", truncate_chars(&title, 22)),
                            None => "目标：未锁定（先点目标输入框）".into(),
                        };
                        ui.label(RichText::new(target_label).size(11.5).color(palette.muted));
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(palette.app_bg)
                    .inner_margin(Margin::same(8.0)),
            )
            .show(ctx, |ui| match self.language {
                Language::Russian => self.show_russian_keyboard(ui, palette),
                Language::Japanese => self.show_japanese_keyboard(ui, palette),
                Language::Korean => self.show_korean_keyboard(ui, palette),
            });
    }

    fn show_russian_keyboard(&mut self, ui: &mut egui::Ui, palette: ui_theme::UiPalette) {
        let gap = 4.0;
        let row_h = ((ui.available_height() - gap * 4.0) / 5.0).clamp(34.0, 52.0);
        let shifted = self.shift_once || self.caps_lock;
        let font_family = self.key_font_family();

        // Row 1: ё + digits + - = (13 keys)
        self.render_char_row(ui, layouts::RU_ROW_1, shifted, row_h, gap, palette);
        ui.add_space(gap);

        // Row 2: 12 Cyrillic keys
        self.render_char_row(ui, layouts::RU_ROW_2, shifted, row_h, gap, palette);
        ui.add_space(gap);

        // Row 3: 11 Cyrillic keys (slightly inset for natural staggered keyboard feel)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let total_w = ui.available_width();
            let inset = 12.0;
            ui.add_space(inset);
            let inner_w = (total_w - (inset + gap) * 2.0).max(100.0);
            let key_w = (inner_w - gap * (layouts::RU_ROW_3.len() - 1) as f32)
                / layouts::RU_ROW_3.len() as f32;
            for key in layouts::RU_ROW_3 {
                if draw_char_key(
                    ui,
                    key,
                    shifted,
                    self.show_hints,
                    Vec2::new(key_w, row_h),
                    palette,
                    &font_family,
                ) {
                    self.handle_char_key(key);
                }
            }
        });
        ui.add_space(gap);

        // Row 4: Shift + 10 keys + Backspace
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let total_w = ui.available_width();
            let units = 1.55 + layouts::RU_ROW_4.len() as f32 + 1.55;
            let count = layouts::RU_ROW_4.len() + 2;
            let unit_w = (total_w - gap * (count - 1) as f32) / units;

            if draw_action_key(
                ui,
                "Shift",
                if self.shift_once { "单次" } else { "" },
                Vec2::new(unit_w * 1.55, row_h),
                KeyTone::Modifier,
                self.shift_once,
                true,
                palette,
            ) {
                self.shift_once = !self.shift_once;
            }

            for key in layouts::RU_ROW_4 {
                if draw_char_key(
                    ui,
                    key,
                    shifted,
                    self.show_hints,
                    Vec2::new(unit_w, row_h),
                    palette,
                    &font_family,
                ) {
                    self.handle_char_key(key);
                }
            }

            if draw_action_key(
                ui,
                "⌫",
                "退格",
                Vec2::new(unit_w * 1.55, row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Backspace);
            }
        });
        ui.add_space(gap);

        // Row 5: Bottom action bar
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let total_w = ui.available_width();
            let weights = [1.4, 0.85, 0.85, 0.85, 4.3, 0.9, 0.9, 1.75];
            let unit_w = (total_w - gap * (weights.len() - 1) as f32) / weights.iter().sum::<f32>();

            if draw_action_key(
                ui,
                "Caps",
                if self.caps_lock { "锁定" } else { "" },
                Vec2::new(unit_w * weights[0], row_h),
                KeyTone::Modifier,
                self.caps_lock,
                true,
                palette,
            ) {
                self.caps_lock = !self.caps_lock;
                if self.caps_lock {
                    self.shift_once = false;
                }
            }
            for (idx, punct) in ["«", "»", "—"].iter().enumerate() {
                if draw_action_key(
                    ui,
                    punct,
                    "",
                    Vec2::new(unit_w * weights[idx + 1], row_h),
                    KeyTone::Punctuation,
                    false,
                    true,
                    palette,
                ) {
                    self.handle_direct_punctuation(punct);
                }
            }
            if draw_action_key(
                ui,
                "空格",
                "Space",
                Vec2::new(unit_w * weights[4], row_h),
                KeyTone::Standard,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Space);
            }
            if draw_action_key(
                ui,
                "←",
                "",
                Vec2::new(unit_w * weights[5], row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Left);
            }
            if draw_action_key(
                ui,
                "→",
                "",
                Vec2::new(unit_w * weights[6], row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Right);
            }
            if draw_action_key(
                ui,
                "回车 ↵",
                "",
                Vec2::new(unit_w * weights[7], row_h),
                KeyTone::Accent,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Enter);
            }
        });
    }

    fn show_japanese_keyboard(&mut self, ui: &mut egui::Ui, palette: ui_theme::UiPalette) {
        let gap = 4.0;
        let row_h = ((ui.available_height() - gap * 5.0) / 6.0).clamp(26.0, 44.0);
        let font_family = self.key_font_family();

        // 5 Gojūon rows: 10 kana columns + 2 utility columns
        let total_w = ui.available_width();
        let util_weight = 1.15;
        let total_units = 10.0 + util_weight * 2.0;
        let unit_w = (total_w - gap * 11.0) / total_units;
        let util_w = unit_w * util_weight;

        for (row_idx, row) in layouts::JA_GOJUON_ROWS.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                for cell in row {
                    if let Some(kana_key) = cell {
                        let ch = kana_key.ch(self.katakana);
                        let mut buf = [0u8; 4];
                        let main_str = ch.encode_utf8(&mut buf);
                        let hint = if self.show_hints { kana_key.romaji } else { "" };
                        let tone = if ch == 'ー' || ch == '・' {
                            KeyTone::Punctuation
                        } else {
                            KeyTone::Standard
                        };
                        if draw_simple_key(
                            ui,
                            main_str,
                            None,
                            hint,
                            Vec2::new(unit_w, row_h),
                            tone,
                            false,
                            palette,
                            &font_family,
                        ) {
                            self.handle_kana_key(ch);
                        }
                    } else {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(unit_w, row_h), Sense::hover());
                        ui.painter().rect_filled(
                            rect,
                            Rounding::same(palette.control_radius),
                            palette.surface_alt.linear_multiply(0.45),
                        );
                    }
                }

                match row_idx {
                    0 => {
                        if draw_action_key(
                            ui,
                            "あ 平",
                            "",
                            Vec2::new(util_w, row_h),
                            KeyTone::Modifier,
                            !self.katakana,
                            true,
                            palette,
                        ) && self.katakana
                        {
                            self.flush_pending();
                            self.katakana = false;
                            self.status = "已切换到平假名（ひらがな）".into();
                            self.status_error = false;
                        }
                        if draw_action_key(
                            ui,
                            "ア 片",
                            "",
                            Vec2::new(util_w, row_h),
                            KeyTone::Modifier,
                            self.katakana,
                            true,
                            palette,
                        ) && !self.katakana
                        {
                            self.flush_pending();
                            self.katakana = true;
                            self.status = "已切换到片假名（カタカナ）".into();
                            self.status_error = false;
                        }
                    }
                    1 => {
                        let can_dakuten = self.kana.can_apply(KanaTransform::Dakuten);
                        if draw_action_key(
                            ui,
                            "゛",
                            "浊音",
                            Vec2::new(util_w, row_h),
                            KeyTone::Modifier,
                            can_dakuten,
                            can_dakuten,
                            palette,
                        ) {
                            self.handle_kana_transform(KanaTransform::Dakuten);
                        }
                        let can_handakuten = self.kana.can_apply(KanaTransform::Handakuten);
                        if draw_action_key(
                            ui,
                            "゜",
                            "半浊",
                            Vec2::new(util_w, row_h),
                            KeyTone::Modifier,
                            can_handakuten,
                            can_handakuten,
                            palette,
                        ) {
                            self.handle_kana_transform(KanaTransform::Handakuten);
                        }
                    }
                    2 => {
                        let can_small = self.kana.can_apply(KanaTransform::Small);
                        if draw_action_key(
                            ui,
                            "小",
                            "促/拗",
                            Vec2::new(util_w, row_h),
                            KeyTone::Modifier,
                            can_small,
                            can_small,
                            palette,
                        ) {
                            self.handle_kana_transform(KanaTransform::Small);
                        }
                        let can_cycle = self.kana.can_apply(KanaTransform::Cycle);
                        if draw_action_key(
                            ui,
                            "゛゜小",
                            "循环",
                            Vec2::new(util_w, row_h),
                            KeyTone::Modifier,
                            can_cycle,
                            can_cycle,
                            palette,
                        ) {
                            self.handle_kana_transform(KanaTransform::Cycle);
                        }
                    }
                    3 => {
                        if draw_action_key(
                            ui,
                            "、",
                            "",
                            Vec2::new(util_w, row_h),
                            KeyTone::Punctuation,
                            false,
                            true,
                            palette,
                        ) {
                            self.handle_direct_punctuation("、");
                        }
                        if draw_action_key(
                            ui,
                            "。",
                            "",
                            Vec2::new(util_w, row_h),
                            KeyTone::Punctuation,
                            false,
                            true,
                            palette,
                        ) {
                            self.handle_direct_punctuation("。");
                        }
                    }
                    _ => {
                        if draw_action_key(
                            ui,
                            "「",
                            "",
                            Vec2::new(util_w, row_h),
                            KeyTone::Punctuation,
                            false,
                            true,
                            palette,
                        ) {
                            self.handle_direct_punctuation("「");
                        }
                        if draw_action_key(
                            ui,
                            "」",
                            "",
                            Vec2::new(util_w, row_h),
                            KeyTone::Punctuation,
                            false,
                            true,
                            palette,
                        ) {
                            self.handle_direct_punctuation("」");
                        }
                    }
                }
            });
            ui.add_space(gap);
        }

        // Row 6: Bottom action bar
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let weights = [0.85, 0.85, 1.15, 4.1, 0.9, 0.9, 1.45, 1.6];
            let unit = (total_w - gap * (weights.len() - 1) as f32) / weights.iter().sum::<f32>();

            if draw_action_key(
                ui,
                "！",
                "",
                Vec2::new(unit * weights[0], row_h),
                KeyTone::Punctuation,
                false,
                true,
                palette,
            ) {
                self.handle_direct_punctuation("！");
            }
            if draw_action_key(
                ui,
                "？",
                "",
                Vec2::new(unit * weights[1], row_h),
                KeyTone::Punctuation,
                false,
                true,
                palette,
            ) {
                self.handle_direct_punctuation("？");
            }
            let has_pending = !self.kana.is_empty();
            if draw_action_key(
                ui,
                "定型",
                "",
                Vec2::new(unit * weights[2], row_h),
                KeyTone::Modifier,
                has_pending,
                has_pending,
                palette,
            ) {
                self.flush_pending();
            }
            if draw_action_key(
                ui,
                "空格",
                "Space",
                Vec2::new(unit * weights[3], row_h),
                KeyTone::Standard,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Space);
            }
            if draw_action_key(
                ui,
                "←",
                "",
                Vec2::new(unit * weights[4], row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Left);
            }
            if draw_action_key(
                ui,
                "→",
                "",
                Vec2::new(unit * weights[5], row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Right);
            }
            if draw_action_key(
                ui,
                "⌫",
                "退格",
                Vec2::new(unit * weights[6], row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Backspace);
            }
            if draw_action_key(
                ui,
                "回车 ↵",
                "",
                Vec2::new(unit * weights[7], row_h),
                KeyTone::Accent,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Enter);
            }
        });
    }

    fn show_korean_keyboard(&mut self, ui: &mut egui::Ui, palette: ui_theme::UiPalette) {
        let gap = 4.0;
        let row_h = ((ui.available_height() - gap * 4.0) / 5.0).clamp(34.0, 52.0);
        let shifted = self.shift_once;
        let font_family = self.key_font_family();

        // Row 1: Digits & punctuation (12 keys)
        self.render_char_row(ui, layouts::KO_ROW_1, shifted, row_h, gap, palette);
        ui.add_space(gap);

        // Row 2: 5 consonants + 5 vowels
        self.render_char_row(ui, layouts::KO_ROW_2, shifted, row_h, gap, palette);
        ui.add_space(gap);

        // Row 3: 5 consonants + 4 vowels (centered slightly like QWERTY home row)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let total_w = ui.available_width();
            let inset = 18.0;
            ui.add_space(inset);
            let inner_w = (total_w - (inset + gap) * 2.0).max(100.0);
            let key_w = (inner_w - gap * (layouts::KO_ROW_3.len() - 1) as f32)
                / layouts::KO_ROW_3.len() as f32;
            for key in layouts::KO_ROW_3 {
                if draw_char_key(
                    ui,
                    key,
                    shifted,
                    self.show_hints,
                    Vec2::new(key_w, row_h),
                    palette,
                    &font_family,
                ) {
                    self.handle_char_key(key);
                }
            }
        });
        ui.add_space(gap);

        // Row 4: Shift + 7 jamo + , . + Backspace
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let total_w = ui.available_width();
            let units = 1.55 + layouts::KO_ROW_4.len() as f32 + 1.55;
            let count = layouts::KO_ROW_4.len() + 2;
            let unit_w = (total_w - gap * (count - 1) as f32) / units;

            if draw_action_key(
                ui,
                "Shift",
                "双辅音",
                Vec2::new(unit_w * 1.55, row_h),
                KeyTone::Modifier,
                self.shift_once,
                true,
                palette,
            ) {
                self.shift_once = !self.shift_once;
            }

            for key in layouts::KO_ROW_4 {
                if draw_char_key(
                    ui,
                    key,
                    shifted,
                    self.show_hints,
                    Vec2::new(unit_w, row_h),
                    palette,
                    &font_family,
                ) {
                    self.handle_char_key(key);
                }
            }

            if draw_action_key(
                ui,
                "⌫",
                "退格",
                Vec2::new(unit_w * 1.55, row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Backspace);
            }
        });
        ui.add_space(gap);

        // Row 5: Bottom action bar
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let total_w = ui.available_width();
            let weights = [1.5, 5.2, 1.0, 1.0, 1.8];
            let unit_w = (total_w - gap * (weights.len() - 1) as f32) / weights.iter().sum::<f32>();
            let has_pending = !self.hangul.is_empty();

            if draw_action_key(
                ui,
                "定型",
                "提交音节",
                Vec2::new(unit_w * weights[0], row_h),
                KeyTone::Modifier,
                has_pending,
                has_pending,
                palette,
            ) {
                self.flush_pending();
            }
            if draw_action_key(
                ui,
                "空格",
                "Space",
                Vec2::new(unit_w * weights[1], row_h),
                KeyTone::Standard,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Space);
            }
            if draw_action_key(
                ui,
                "←",
                "",
                Vec2::new(unit_w * weights[2], row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Left);
            }
            if draw_action_key(
                ui,
                "→",
                "",
                Vec2::new(unit_w * weights[3], row_h),
                KeyTone::Modifier,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Right);
            }
            if draw_action_key(
                ui,
                "回车 ↵",
                "",
                Vec2::new(unit_w * weights[4], row_h),
                KeyTone::Accent,
                false,
                true,
                palette,
            ) {
                self.handle_control_key(ControlKey::Enter);
            }
        });
    }

    fn render_char_row(
        &mut self,
        ui: &mut egui::Ui,
        row: &[CharKey],
        shifted: bool,
        row_h: f32,
        gap: f32,
        palette: ui_theme::UiPalette,
    ) {
        let font_family = self.key_font_family();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            let total_w = ui.available_width();
            let key_w = (total_w - gap * (row.len() - 1) as f32) / row.len() as f32;
            for key in row {
                if draw_char_key(
                    ui,
                    key,
                    shifted,
                    self.show_hints,
                    Vec2::new(key_w, row_h),
                    palette,
                    &font_family,
                ) {
                    self.handle_char_key(key);
                }
            }
        });
    }
}

impl eframe::App for KeyboardApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        no_activate::ensure_no_activate(frame);
        self.show(ctx);
    }
}

fn draw_char_key(
    ui: &mut egui::Ui,
    key: &CharKey,
    shifted: bool,
    show_hints: bool,
    size: Vec2,
    palette: ui_theme::UiPalette,
    font_family: &FontFamily,
) -> bool {
    let main = key.active_text(shifted);
    let alt = key.alternate_text(shifted);
    let hint = if show_hints {
        key.active_hint(shifted)
    } else {
        ""
    };
    draw_simple_key(
        ui,
        main,
        alt,
        hint,
        size,
        key.tone,
        false,
        palette,
        font_family,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_simple_key(
    ui: &mut egui::Ui,
    main: &str,
    alt: Option<&str>,
    hint: &str,
    size: Vec2,
    tone: KeyTone,
    active: bool,
    palette: ui_theme::UiPalette,
    font_family: &FontFamily,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (bg, border, text_color) = key_colors(
        tone,
        active,
        response.hovered(),
        response.is_pointer_button_down_on(),
        true,
        palette,
    );
    let painter = ui.painter_at(rect);
    let rounding = Rounding::same(palette.control_radius);

    painter.rect_filled(rect, rounding, bg);
    painter.rect_stroke(rect, rounding, Stroke::new(palette.border_width, border));

    let has_hint = !hint.is_empty();
    let main_size = if has_hint {
        (size.y * 0.44).clamp(14.0, 20.0)
    } else {
        (size.y * 0.48).clamp(15.0, 22.0)
    };
    let main_y = if has_hint {
        rect.center().y - size.y * 0.11
    } else {
        rect.center().y
    };
    painter.text(
        Pos2::new(rect.center().x, main_y),
        Align2::CENTER_CENTER,
        main,
        FontId::new(main_size, font_family.clone()),
        text_color,
    );

    if let Some(alt_text) = alt {
        painter.text(
            Pos2::new(rect.left() + 4.5, rect.top() + 3.0),
            Align2::LEFT_TOP,
            alt_text,
            FontId::new((size.y * 0.24).clamp(9.0, 11.5), font_family.clone()),
            palette.muted,
        );
    }

    if has_hint {
        painter.text(
            Pos2::new(rect.center().x, rect.bottom() - 3.0),
            Align2::CENTER_BOTTOM,
            hint,
            FontId::proportional((size.y * 0.23).clamp(9.0, 11.0)),
            palette.muted,
        );
    }

    response.clicked()
}

#[allow(clippy::too_many_arguments)]
fn draw_action_key(
    ui: &mut egui::Ui,
    label: &str,
    sublabel: &str,
    size: Vec2,
    tone: KeyTone,
    active: bool,
    enabled: bool,
    palette: ui_theme::UiPalette,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let (bg, border, text_color) = key_colors(
        tone,
        active,
        enabled && response.hovered(),
        enabled && response.is_pointer_button_down_on(),
        enabled,
        palette,
    );
    let painter = ui.painter_at(rect);
    let rounding = Rounding::same(palette.control_radius);

    painter.rect_filled(rect, rounding, bg);
    painter.rect_stroke(rect, rounding, Stroke::new(palette.border_width, border));

    let has_sub = !sublabel.is_empty();
    let main_size = (size.y * 0.34).clamp(11.5, 14.5);
    let main_y = if has_sub {
        rect.center().y - size.y * 0.13
    } else {
        rect.center().y
    };
    painter.text(
        Pos2::new(rect.center().x, main_y),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(main_size),
        text_color,
    );

    if has_sub {
        painter.text(
            Pos2::new(rect.center().x, rect.bottom() - 3.5),
            Align2::CENTER_BOTTOM,
            sublabel,
            FontId::proportional((size.y * 0.22).clamp(8.5, 10.5)),
            if active && tone == KeyTone::Modifier {
                palette.accent
            } else {
                palette.muted
            },
        );
    }

    enabled && response.clicked()
}

fn key_colors(
    tone: KeyTone,
    active: bool,
    hovered: bool,
    pressed: bool,
    enabled: bool,
    palette: ui_theme::UiPalette,
) -> (Color32, Color32, Color32) {
    if !enabled {
        return (
            palette.surface_alt,
            palette.border_subtle,
            palette.muted.linear_multiply(0.55),
        );
    }
    if active {
        return match tone {
            KeyTone::Accent => (palette.accent_pressed, palette.accent, palette.accent_text),
            _ => (palette.accent_soft, palette.accent, palette.accent),
        };
    }
    if pressed {
        return match tone {
            KeyTone::Accent => (palette.accent_pressed, palette.accent, palette.accent_text),
            _ => (palette.accent_soft, palette.accent, palette.text),
        };
    }
    if hovered {
        return match tone {
            KeyTone::Accent => (palette.accent_hover, palette.accent, palette.accent_text),
            _ => (palette.control_hover, palette.accent, palette.text),
        };
    }
    match tone {
        KeyTone::Standard => (palette.surface, palette.border, palette.text),
        KeyTone::Consonant => (palette.surface, palette.border, palette.text),
        KeyTone::Vowel => (palette.info_soft, palette.border, palette.text),
        KeyTone::Modifier => (palette.control_bg, palette.border, palette.text),
        KeyTone::Punctuation => (palette.surface_alt, palette.border_subtle, palette.text),
        KeyTone::Accent => (palette.accent, palette.accent, palette.accent_text),
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}…")
    } else {
        collected
    }
}

fn parse_initial_language() -> Language {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--lang" {
            if let Some(value) = args.next() {
                return match value.trim().to_ascii_lowercase().as_str() {
                    "ja" | "jp" | "japanese" => Language::Japanese,
                    "ko" | "kr" | "korean" => Language::Korean,
                    _ => Language::Russian,
                };
            }
        }
    }
    Language::Japanese
}

#[cfg(windows)]
fn copy_text(text: &str) -> Result<(), String> {
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text)
        .map_err(|e| format!("复制失败：{e}"))
}

#[cfg(not(windows))]
fn copy_text(_text: &str) -> Result<(), String> {
    Err("复制仅支持 Windows".into())
}

pub(super) fn run() -> eframe::Result<()> {
    pinyin_ime::windows_security::apply_process_hardening();
    let tracker = win_target::Tracker::new();
    let initial_lang = parse_initial_language();
    let Ok(Some(_instance)) = win_single_instance::claim_or_activate_existing(
        "Local\\KaixinInput_Keyboard_v1",
        WINDOW_TITLE,
    ) else {
        return Ok(());
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size(DEFAULT_SIZE)
            .with_min_inner_size(MIN_SIZE)
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(KeyboardApp::new(cc, tracker, initial_lang)))),
    )
}
