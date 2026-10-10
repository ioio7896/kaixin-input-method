#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "srf_ime_symbols/catalog.rs"]
mod catalog;
#[path = "../fonts.rs"]
mod fonts;
#[path = "srf_ime_symbols/target.rs"]
mod target;

use eframe::egui::{self, FontId, Frame, Margin, RichText};
use pinyin_ime::{ui_theme, win_paste, win_single_instance};
use std::time::Duration;

const WINDOW_TITLE: &str = "开心输入法 符号大全";
const DEFAULT_SIZE: [f32; 2] = [780.0, 760.0];
const MIN_SIZE: [f32; 2] = [670.0, 660.0];
const SEARCH_ID: &str = "symbol_search";

struct SymbolsApp {
    symbols: Vec<catalog::Symbol>,
    categories: Vec<&'static str>,
    category: &'static str,
    query: String,
    filtered: Vec<usize>,
    page: usize,
    recent: Vec<usize>,
    selected: Option<usize>,
    copy_only: bool,
    status: String,
    status_error: bool,
    _target_tracker: target::Tracker,
}

impl SymbolsApp {
    fn new(cc: &eframe::CreationContext<'_>, tracker: target::Tracker) -> Self {
        fonts::install_cjk_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        ui_theme::apply_tool_style(&cc.egui_ctx);
        let symbols = catalog::load();
        let categories = catalog::categories(&symbols);
        let filtered = catalog::filter(&symbols, Some("常用"), "");
        Self {
            symbols,
            categories,
            category: "常用",
            query: String::new(),
            filtered,
            page: 0,
            recent: Vec::new(),
            selected: None,
            copy_only: false,
            status: "先点击目标编辑框，再点选符号；窗口保持置顶，可连续输入。".into(),
            status_error: false,
            _target_tracker: tracker,
        }
    }

    fn refresh_filter(&mut self) {
        // Searching covers the whole catalog, so professional symbols are easy
        // to discover without guessing their category first.
        self.filtered = if !self.query.trim().is_empty() {
            catalog::filter(&self.symbols, None, &self.query)
        } else if self.category == "最近使用" {
            self.recent.clone()
        } else {
            catalog::filter(
                &self.symbols,
                (self.category != "全部").then_some(self.category),
                "",
            )
        };
        self.page = 0;
    }

    fn choose_category(&mut self, category: &'static str) {
        self.category = category;
        self.query.clear();
        self.refresh_filter();
    }

    fn insert(&mut self, index: usize) {
        self.selected = Some(index);
        let symbol = &self.symbols[index];
        let result = if self.copy_only {
            copy_symbol(symbol.text)
        } else {
            target::validated_target()
                .and_then(|hwnd| win_paste::send_unicode_text_to_target(hwnd, symbol.text))
        };
        match result {
            Ok(()) => {
                self.status = format!(
                    "{} {} · {}",
                    if self.copy_only {
                        "已复制"
                    } else {
                        "已输入"
                    },
                    symbol.text,
                    symbol.name
                );
                self.status_error = false;
                self.recent.retain(|&i| self.symbols[i].text != symbol.text);
                self.recent.insert(0, index);
                self.recent.truncate(catalog::PAGE_SIZE);
                if self.category == "最近使用" && self.query.trim().is_empty() {
                    self.refresh_filter();
                }
            }
            Err(error) => {
                self.status = error;
                self.status_error = true;
            }
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if !ctx.memory(|memory| memory.has_focus(egui::Id::new(SEARCH_ID))) {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::PageDown)) {
                self.page = (self.page + 1).min(catalog::page_count(self.filtered.len()) - 1);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::PageUp)) {
                self.page = self.page.saturating_sub(1);
            }
        }
    }
}

impl SymbolsApp {
    fn show(&mut self, ctx: &egui::Context) {
        // Fallback for a missed WinEvent; never adopt this tool or the shell.
        target::remember_foreground();
        ctx.request_repaint_after(Duration::from_millis(200));
        self.keyboard(ctx);
        let palette = ui_theme::UiPalette::from_visuals(&ctx.style().visuals);

        egui::TopBottomPanel::top("symbols_header")
            .frame(
                Frame::none()
                    .fill(palette.surface)
                    .inner_margin(Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("符号大全");
                    ui.label(RichText::new("10 × 10 软键盘").small().color(palette.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.copy_only, "仅复制")
                            .on_hover_text("点选符号后复制到剪贴板，按 Ctrl+V 粘贴。");
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let response = ui.add_sized(
                        [ui.available_width() - 65.0, 32.0],
                        egui::TextEdit::singleline(&mut self.query)
                            .id(egui::Id::new(SEARCH_ID))
                            .hint_text("搜索全部：积分 / jifen / alpha / 硫酸根 / U+03B1"),
                    );
                    if response.changed() {
                        self.refresh_filter();
                    }
                    if ui.button("清除").clicked() {
                        self.query.clear();
                        self.refresh_filter();
                    }
                });
            });

        egui::TopBottomPanel::bottom("symbols_status")
            .frame(
                Frame::none()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(14.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(&self.status)
                        .size(13.0)
                        .color(if self.status_error {
                            palette.danger
                        } else {
                            palette.muted
                        }),
                );
                ui.label(
                    RichText::new(
                        "置顶临时工具 · PgUp / PgDn 翻页 · Esc 关闭 · 专业符号无需启用词库",
                    )
                    .small()
                    .color(palette.muted),
                );
            });

        egui::SidePanel::left("symbol_categories")
            .exact_width(126.0)
            .resizable(false)
            .frame(
                Frame::none()
                    .fill(palette.nav_bg)
                    .inner_margin(Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("分类").strong().color(palette.muted));
                ui.add_space(6.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let categories = self.categories.clone();
                    for category in ["最近使用", "全部"].into_iter().chain(categories) {
                        let selected = self.category == category && self.query.trim().is_empty();
                        let response = ui.add_sized(
                            [ui.available_width(), 30.0],
                            egui::SelectableLabel::new(selected, category),
                        );
                        if response.clicked() {
                            self.choose_category(category);
                        }
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(palette.app_bg)
                    .inner_margin(Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                let pages = catalog::page_count(self.filtered.len());
                self.page = self.page.min(pages - 1);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if self.query.trim().is_empty() {
                            self.category
                        } else {
                            "搜索结果"
                        })
                        .strong(),
                    );
                    ui.label(
                        RichText::new(format!("{} 项", self.filtered.len()))
                            .small()
                            .color(palette.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(self.page + 1 < pages, egui::Button::new("下一页"))
                            .clicked()
                        {
                            self.page += 1;
                        }
                        ui.label(format!("{} / {pages}", self.page + 1));
                        if ui
                            .add_enabled(self.page > 0, egui::Button::new("上一页"))
                            .clicked()
                        {
                            self.page -= 1;
                        }
                    });
                });
                ui.add_space(10.0);
                let start = self.page * catalog::PAGE_SIZE;
                let visible = &self.filtered[start.min(self.filtered.len())
                    ..(start + catalog::PAGE_SIZE).min(self.filtered.len())];
                let (clicked, hovered) = symbol_grid(ui, &self.symbols, visible, self.selected);
                ui.add_space(10.0);
                if let Some(index) = hovered.or(self.selected) {
                    let symbol = &self.symbols[index];
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(symbol.text)
                                .font(FontId::proportional(27.0))
                                .color(palette.accent),
                        );
                        ui.vertical(|ui| {
                            ui.label(RichText::new(symbol.name).size(13.0));
                            ui.label(RichText::new(&symbol.code).size(11.0).color(palette.muted));
                        });
                    });
                } else {
                    ui.label(
                        RichText::new(if self.filtered.is_empty() {
                            "暂无符号，请调整搜索或选择其他分类。"
                        } else {
                            "将鼠标移到符号上，查看名称和 Unicode 编码。"
                        })
                        .small()
                        .color(palette.muted),
                    );
                }
                if let Some(index) = clicked {
                    self.insert(index);
                }
            });
    }
}

impl eframe::App for SymbolsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.show(ctx);
    }
}

/// Always draw 100 equal slots, including disabled blanks on the last page.
fn symbol_grid(
    ui: &mut egui::Ui,
    symbols: &[catalog::Symbol],
    visible: &[usize],
    selected: Option<usize>,
) -> (Option<usize>, Option<usize>) {
    let spacing = 4.0;
    let width = (ui.available_width() - spacing * 9.0) / catalog::COLUMNS as f32;
    let height =
        ((ui.available_height() - 92.0 - spacing * 9.0) / catalog::ROWS as f32).clamp(32.0, 52.0);
    let palette = ui_theme::UiPalette::from_visuals(ui.visuals());
    let mut clicked = None;
    let mut hovered = None;
    egui::Grid::new("symbols_10_by_10")
        .spacing(egui::vec2(spacing, spacing))
        .min_col_width(width)
        .max_col_width(width)
        .show(ui, |ui| {
            ui.spacing_mut().button_padding = egui::vec2(1.0, 1.0);
            for slot in 0..catalog::PAGE_SIZE {
                if let Some(&index) = visible.get(slot) {
                    let symbol = &symbols[index];
                    let supported = ui
                        .fonts(|fonts| fonts.has_glyphs(&FontId::proportional(24.0), symbol.text));
                    let fallback = symbol.code.replace("U+", "");
                    let label = if supported { symbol.text } else { &fallback };
                    let measured_width = ui.fonts(|fonts| {
                        fonts
                            .layout_no_wrap(
                                label.to_owned(),
                                FontId::proportional(24.0),
                                palette.text,
                            )
                            .size()
                            .x
                    });
                    let font_size = (24.0 * (width - 6.0) / measured_width.max(1.0))
                        .clamp(9.0, if supported { 24.0 } else { 12.0 })
                        .min(height * 0.65);
                    let button = egui::Button::new(
                        RichText::new(label).font(FontId::proportional(font_size)),
                    )
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .fill(if selected == Some(index) {
                        palette.accent_soft
                    } else {
                        palette.surface
                    });
                    let response = ui.add_sized([width, height], button);
                    if response.hovered() {
                        hovered = Some(index);
                    }
                    if response.clicked() {
                        clicked = Some(index);
                    }
                    response.on_hover_text(format!(
                        "{}\n{}\n{}{}",
                        symbol.name,
                        symbol.category,
                        symbol.code,
                        if supported {
                            ""
                        } else {
                            "\n本机字体缺少此字形，按编码显示；仍可输入或复制。"
                        }
                    ));
                } else {
                    ui.add_enabled_ui(false, |ui| {
                        ui.add_sized([width, height], egui::Button::new(""));
                    });
                }
                if slot % catalog::COLUMNS == catalog::COLUMNS - 1 {
                    ui.end_row();
                }
            }
        });
    (clicked, hovered)
}

#[cfg(windows)]
fn copy_symbol(text: &str) -> Result<(), String> {
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text)
        .map_err(|e| format!("复制失败：{e}"))
}
#[cfg(not(windows))]
fn copy_symbol(_text: &str) -> Result<(), String> {
    Err("复制符号仅支持 Windows".into())
}

fn main() -> eframe::Result<()> {
    pinyin_ime::windows_security::apply_process_hardening();
    let tracker = target::Tracker::new();
    let Ok(Some(_instance)) = win_single_instance::claim_or_activate_existing(
        "Local\\KaixinInput_Symbols_v1",
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
        Box::new(move |cc| Ok(Box::new(SymbolsApp::new(cc, tracker)))),
    )
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    #[test]
    fn page_keys_work_after_selecting_a_key_but_leave_search_editing_alone() {
        let ctx = egui::Context::default();
        let symbols = catalog::load();
        let mut app = SymbolsApp {
            categories: catalog::categories(&symbols),
            symbols,
            category: "数学运算",
            query: String::new(),
            filtered: Vec::new(),
            page: 0,
            recent: Vec::new(),
            selected: None,
            copy_only: false,
            status: String::new(),
            status_error: false,
            _target_tracker: target::Tracker::for_test(),
        };
        app.refresh_filter();
        for (focus, expected_page) in [("a_symbol_key", 1), (SEARCH_ID, 1), ("a_category", 2)] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::PageDown,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(focus)));
                    app.keyboard(ctx);
                },
            );
            assert_eq!(app.page, expected_page, "focus: {focus}");
        }
    }

    #[test]
    fn full_window_renders_ten_equal_rows_and_columns_at_minimum_and_default_size() {
        for (label, size, category, query) in [
            ("common", DEFAULT_SIZE, "常用", ""),
            ("minimum-math", MIN_SIZE, "数学运算", ""),
            ("chemistry", MIN_SIZE, "化学", ""),
            ("search", MIN_SIZE, "常用", "硫酸根"),
            ("empty", MIN_SIZE, "最近使用", ""),
        ] {
            let ctx = egui::Context::default();
            fonts::install_cjk_fonts(&ctx);
            ctx.set_visuals(egui::Visuals::light());
            ui_theme::apply_tool_style(&ctx);
            let symbols = catalog::load();
            let mut app = SymbolsApp {
                categories: catalog::categories(&symbols),
                symbols,
                category,
                query: query.into(),
                filtered: Vec::new(),
                page: 0,
                recent: Vec::new(),
                selected: None,
                copy_only: false,
                status: "先点击目标编辑框，再点选符号；窗口保持置顶，可连续输入。".into(),
                status_error: false,
                _target_tracker: target::Tracker::for_test(),
            };
            app.refresh_filter();
            let mut textures = Vec::new();
            let mut output = egui::FullOutput::default();
            for _ in 0..3 {
                output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(size[0], size[1]),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.show(ctx),
                );
                for (id, delta) in &output.textures_delta.set {
                    let pixels: Vec<[u8; 4]> = match &delta.image {
                        egui::ImageData::Color(image) => {
                            image.pixels.iter().map(|p| p.to_array()).collect()
                        }
                        egui::ImageData::Font(image) => {
                            image.srgba_pixels(None).map(|p| p.to_array()).collect()
                        }
                    };
                    textures.push(serde_json::json!({"id": format!("{id:?}"), "size": delta.image.size(), "pos": delta.pos, "pixels": pixels}));
                }
            }
            let mut groups = std::collections::BTreeMap::<(i32, i32), Vec<egui::Rect>>::new();
            for shape in &output.shapes {
                if let egui::epaint::Shape::Rect(rect) = &shape.shape {
                    let size = rect.rect.size();
                    if (30.0..80.0).contains(&size.x) && (32.0..53.0).contains(&size.y) {
                        groups
                            .entry((
                                (size.x * 10.0).round() as i32,
                                (size.y * 10.0).round() as i32,
                            ))
                            .or_default()
                            .push(rect.rect);
                    }
                }
            }
            let keys = groups
                .values()
                .find(|rects| rects.len() == 100)
                .unwrap_or_else(|| {
                    panic!(
                        "{label}: not 100 equal keys: {:?}",
                        groups.iter().map(|(k, v)| (k, v.len())).collect::<Vec<_>>()
                    )
                });
            let xs: std::collections::BTreeSet<_> =
                keys.iter().map(|r| r.left().round() as i32).collect();
            let ys: std::collections::BTreeSet<_> =
                keys.iter().map(|r| r.top().round() as i32).collect();
            assert_eq!(xs.len(), 10, "{label}: columns");
            assert_eq!(ys.len(), 10, "{label}: rows");
            assert!(
                keys.iter().all(|r| r.left() >= 0.0
                    && r.right() <= size[0]
                    && r.top() >= 0.0
                    && r.bottom() <= size[1]),
                "{label}: keys leave viewport"
            );
            if let Ok(dir) = std::env::var("KAIXIN_SYMBOLS_RENDER_DIR") {
                let meshes: Vec<_> = ctx.tessellate(output.shapes, output.pixels_per_point).into_iter().filter_map(|primitive| {
                    if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                        Some(serde_json::json!({"texture": format!("{:?}", mesh.texture_id), "clip": [primitive.clip_rect.left(), primitive.clip_rect.top(), primitive.clip_rect.right(), primitive.clip_rect.bottom()], "indices": mesh.indices, "vertices": mesh.vertices.iter().map(|v| (v.pos.x, v.pos.y, v.uv.x, v.uv.y, v.color.to_array())).collect::<Vec<_>>()}))
                    } else { None }
                }).collect();
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    std::path::Path::new(&dir).join(format!("{label}.json")),
                    serde_json::to_vec(
                        &serde_json::json!({"size": size, "textures": textures, "meshes": meshes}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
    }
}
