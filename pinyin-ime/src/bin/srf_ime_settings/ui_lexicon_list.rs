use super::*;

pub(super) fn set_lexicon_group_enabled(
    states: &mut BTreeMap<String, bool>,
    tags: &[String],
    enabled: bool,
) {
    for tag in tags {
        if let Some(state) = states.get_mut(tag) {
            *state = enabled;
        }
    }
}

pub(super) fn lexicon_display_group(tag: &str) -> &'static str {
    match pinyin_ime::lexicon_prefs::optional_lexicon_group(tag) {
        "日常表达" | "成语与固定表达" => "日常生活",
        "办公与办事" => "办公沟通",
        "科技与开发" => "科技开发",
        "专业领域" | "药物名称" => "专业领域",
        "名称与实体"
        | "名称与实体 · 按需启用"
        | "名称与实体 · 低优先级"
        | "地区词库 · 全国与世界"
        | "地区词库 · 杭州" => "名称与地区",
        "通用词语补充" | "通用补充 · 待筛" => "通用补充",
        _ => "其他扩展",
    }
}

pub(super) fn lexicon_sort_key(tag: &str) -> (&str, &str) {
    (
        tag.strip_prefix("category_")
            .or_else(|| tag.strip_prefix("professional_"))
            .unwrap_or(tag),
        tag,
    )
}

pub(super) fn lexicon_coverage(tag: &str) -> &'static str {
    if tag.starts_with("professional_") {
        "精选学科基础术语，与同领域分类补充词库独立开关，可能有重合；默认按需启用。"
    } else if tag.starts_with("category_") {
        "按具体场景或领域整理的补充词库；可能与同领域专业基础术语重合，可独立启用。"
    } else {
        pinyin_ime::lexicon_prefs::optional_lexicon_description(tag)
    }
}

pub(super) fn lexicon_group_hint(group: &str) -> &'static str {
    match group {
        "日常生活" => "生活、购物、出行、聊天和固定表达。",
        "办公沟通" => "办公协作、项目、人事与政务办事。",
        "科技开发" => "软件、设备、编程与技术交流。",
        "专业领域" => "行业、学科及药物词库，按需要选择。",
        "名称与地区" => "人物、机构、品牌与国内外和本地地名。",
        "通用补充" => "待筛选的通用词语补充，按需启用。",
        _ => "其他补充词库；可在详情中查看用途与来源。",
    }
}

fn lexicon_name(ui: &mut egui::Ui, title: &str, enabled: &mut bool, help: &str) {
    ui.horizontal_top(|ui| {
        let response = ui.checkbox(enabled, "").on_hover_text(help);
        let mut job = egui::WidgetText::from(title).into_layout_job(
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Min,
        );
        job.wrap.break_anywhere = true;
        let label = ui
            .add(egui::Label::new(job).wrap().sense(egui::Sense::click()))
            .on_hover_text(help);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *enabled, title)
        });
        if label.clicked() {
            *enabled = !*enabled;
        }
    });
}

fn lexicon_count(ui: &mut egui::Ui, count: &str) {
    ui.label(RichText::new(count).small().color(fluent_palette(ui).muted))
        .on_hover_text("按源文件非注释条目统计，同词不同读音分别计数；不代表净新增词数。");
}

fn lexicon_details(ui: &mut egui::Ui, description: &str, source: &str) {
    ui.menu_button("详情", |ui| {
        ui.set_max_width(360.0);
        ui.add(egui::Label::new(description).wrap());
        ui.separator();
        ui.strong("来源");
        // Allow long paths and identifiers to wrap inside the menu.
        let mut job = egui::WidgetText::from(source).into_layout_job(
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Min,
        );
        job.wrap.break_anywhere = true;
        ui.add(egui::Label::new(job).wrap());
    });
}

pub(super) fn lexicon_list_row(
    ui: &mut egui::Ui,
    title: &str,
    count: &str,
    enabled: &mut bool,
    description: &str,
    source: &str,
) {
    let width = ui.available_width();
    let gap = ui.spacing().item_spacing.x.max(8.0);
    let count_width = 90.0;
    let details_width = 54.0;
    let name_width = width - count_width - details_width - gap * 2.0;
    let help = format!("{description}\n来源：{source}");
    let response = ui
        .push_id(title, |ui| {
            if name_width < 140.0 {
                ui.vertical(|ui| {
                    lexicon_name(ui, title, enabled, &help);
                    ui.horizontal(|ui| {
                        lexicon_count(ui, count);
                        lexicon_details(ui, description, source);
                    });
                });
            } else {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(
                        egui::vec2(name_width, 28.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(name_width);
                            lexicon_name(ui, title, enabled, &help);
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(count_width, 28.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| lexicon_count(ui, count),
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(details_width, 28.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| lexicon_details(ui, description, source),
                    );
                });
            }
        })
        .response;
    setting_row_help(response, &help);
}
