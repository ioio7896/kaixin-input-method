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

pub(super) fn lexicon_group_hint(group: &str) -> &'static str {
    match group {
        "日常表达" => "生活、购物、办公与常用表达，按使用习惯选择。",
        "科技与开发" => "软件、设备、编程与技术交流用语。",
        "专业领域" => "各行业与学科术语，按专业需要选择。",
        "名称与实体" => "人物、机构、品牌等名称。",
        "药物名称" => "药品名称与相关词条。",
        "地区词库 · 全国与世界" => "行政区划、城市与世界地名。",
        "地区词库 · 杭州" => "杭州本地地名与生活用语。",
        _ => "其他补充词库；可在详情中查看用途与来源。",
    }
}

fn lexicon_name(ui: &mut egui::Ui, title: &str, enabled: &mut bool) {
    ui.horizontal_top(|ui| {
        let response = ui.checkbox(enabled, "").on_hover_text(title);
        let mut job = egui::WidgetText::from(title).into_layout_job(
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Min,
        );
        job.wrap.break_anywhere = true;
        let label = ui.add(egui::Label::new(job).wrap().sense(egui::Sense::click()));
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
    ui.push_id(title, |ui| {
        if name_width < 140.0 {
            ui.vertical(|ui| {
                lexicon_name(ui, title, enabled);
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
                        lexicon_name(ui, title, enabled);
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
    });
}
