//! Settings page controls.
use super::*;

pub(super) fn advanced_rank_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let defaults = SettingsModel::default();
    let customized = app.model.w_single_lm != defaults.w_single_lm
        || app.model.w_phrase_path != defaults.w_phrase_path
        || app.model.lm_single_scale != defaults.lm_single_scale;
    section_panel(ui, "候选排序权重", |ui| {
        ui.horizontal(|ui| {
            if outline_button(ui, "恢复排序推荐").clicked() {
                app.reset_rank_defaults();
            }
            status_badge(
                ui,
                if customized {
                    StatusTone::Warning
                } else {
                    StatusTone::Success
                },
                if customized {
                    "已自定义"
                } else {
                    "推荐值"
                },
            );
        });
        setting_slider_f64(
            ui,
            "单字权重",
            "单字语言模型对排序的影响。",
            &mut app.model.w_single_lm,
            0.0..=1.0,
        );
        setting_slider_f64(
            ui,
            "词组权重",
            "词组路径得分对排序的影响。",
            &mut app.model.w_phrase_path,
            0.0..=1.0,
        );
        setting_slider_f64(
            ui,
            "单字缩放",
            "放大或压低单字语言模型分值。",
            &mut app.model.lm_single_scale,
            0.1..=30.0,
        );
    });
}

pub(super) fn advanced_engine_tuning_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let defaults = SettingsModel::default();
    let customized = app.model.prefix_cache_capacity != defaults.prefix_cache_capacity
        || app.model.final_lookup_cache_capacity != defaults.final_lookup_cache_capacity
        || app.model.short_lookup_cache_capacity != defaults.short_lookup_cache_capacity
        || app.model.long_lookup_soft_budget_ms != defaults.long_lookup_soft_budget_ms
        || app.model.long_lookup_min_first_batch_candidates
            != defaults.long_lookup_min_first_batch_candidates;
    section_panel(ui, "引擎与缓存调优", |ui| {
        ui.horizontal(|ui| {
            if outline_button(ui, "恢复性能推荐").clicked() {
                app.reset_engine_tuning_defaults();
            }
            status_badge(
                ui,
                if customized {
                    StatusTone::Warning
                } else {
                    StatusTone::Success
                },
                if customized {
                    "已自定义"
                } else {
                    "推荐值"
                },
            );
        });
        ui.label(
            RichText::new(
                "长输入先返回首屏候选；纠错和完整重排只在首屏后继续，避免连续输入被慢路径阻塞。",
            )
            .small()
            .color(fluent_palette(ui).muted),
        );
        setting_slider_usize(
            ui,
            "前缀缓存容量",
            "影响连续输入时前缀词库查询的复用数量。",
            &mut app.model.prefix_cache_capacity,
            8..=512,
        );
        setting_slider_usize(
            ui,
            "最终查询缓存容量",
            "缓存完整候选基础结果；选择反馈会在命中后重新轻量应用。",
            &mut app.model.final_lookup_cache_capacity,
            8..=512,
        );
        setting_slider_usize(
            ui,
            "短输入缓存容量",
            "缓存短拼音/简拼输入结果。",
            &mut app.model.short_lookup_cache_capacity,
            8..=512,
        );
        setting_slider_usize(
            ui,
            "长输入软预算 ms",
            "首屏候选的软时间预算；4 ms 为速度优先，调高会减少部分结果但增加首屏等待。",
            &mut app.model.long_lookup_soft_budget_ms,
            1..=50,
        );
        setting_slider_usize(
            ui,
            "首批候选下限",
            "达到这个数量后才触发首屏截止；速度优先可设为 6。",
            &mut app.model.long_lookup_min_first_batch_candidates,
            1..=128,
        );
    });
}
