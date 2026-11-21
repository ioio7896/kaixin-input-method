//! Settings page controls.
use super::*;

pub(super) fn input_habits_ui(ui: &mut egui::Ui, model: &mut SettingsModel) {
    section_panel(ui, "模式默认值", |ui| {
        setting_toggle(
            ui,
            "默认英文模式",
            "打开输入法时默认进入英文直输。",
            &mut model.default_ascii,
        );
        setting_toggle(
            ui,
            "全局英文模式",
            "中英状态在所有应用之间共享。",
            &mut model.global_ascii,
        );
        setting_toggle(
            ui,
            "中文拼音模式采用半角字符",
            "字母、数字和符号保留半角形式，如 - 而非 －；优先于全角和中文标点设置，符号补全仍可用。",
            &mut model.chinese_halfwidth,
        );
        setting_toggle(
            ui,
            "中文模式下键入全角字符",
            "中文模式启动后默认使用全角字母和符号；Shift+Space 可临时切换。",
            &mut model.default_full_shape,
        );
        setting_toggle(
            ui,
            "默认中文标点",
            "启动后默认把常用标点转换为中文标点。",
            &mut model.default_chinese_punct,
        );
        setting_toggle(
            ui,
            "中文标点使用弯引号",
            "中文引号优先输出成“”和‘’。",
            &mut model.curly_punct,
        );
        setting_toggle(
            ui,
            "开启符号自动补全",
            "输入括号或引号时自动补齐成一对，并把光标放到中间。",
            &mut model.auto_pair_punct,
        );
        setting_toggle(
            ui,
            "数字全角",
            "中文模式下把数字键转换为全角数字。",
            &mut model.number_fullwidth,
        );
        setting_toggle(
            ui,
            "符号全角",
            "拼音模式下把括号、斜杠和 ASCII 符号转换为全角符号。",
            &mut model.symbol_fullwidth,
        );
        setting_toggle(
            ui,
            "Shift 符号临时英文直出",
            "按住 Shift 输入数字和符号时保留键盘原始 ASCII 符号。",
            &mut model.shift_symbol_temporary_ascii,
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "拼音能力", |ui| {
        setting_toggle(
            ui,
            "默认模糊音",
            "启动后默认启用 z/zh、s/sh 等模糊匹配。",
            &mut model.default_fuzzy_pinyin,
        );
        ui.add_enabled_ui(model.default_fuzzy_pinyin, |ui| {
            ui.collapsing("模糊音细项", |ui| {
                setting_toggle(
                    ui,
                    "z / zh",
                    "允许 z 和 zh 双向模糊匹配。",
                    &mut model.fuzzy_zh_z,
                );
                setting_toggle(
                    ui,
                    "c / ch",
                    "允许 c 和 ch 双向模糊匹配。",
                    &mut model.fuzzy_ch_c,
                );
                setting_toggle(
                    ui,
                    "s / sh",
                    "允许 s 和 sh 双向模糊匹配。",
                    &mut model.fuzzy_sh_s,
                );
                setting_toggle(
                    ui,
                    "n / l",
                    "允许 n 和 l 双向模糊匹配。",
                    &mut model.fuzzy_n_l,
                );
                setting_toggle(
                    ui,
                    "f / h",
                    "允许 f 和 h 双向模糊匹配。",
                    &mut model.fuzzy_f_h,
                );
                setting_toggle(
                    ui,
                    "an / ang",
                    "允许 an 和 ang 双向模糊匹配。",
                    &mut model.fuzzy_an_ang,
                );
                setting_toggle(
                    ui,
                    "en / eng",
                    "允许 en 和 eng 双向模糊匹配。",
                    &mut model.fuzzy_en_eng,
                );
                setting_toggle(
                    ui,
                    "in / ing",
                    "允许 in 和 ing 双向模糊匹配。",
                    &mut model.fuzzy_in_ing,
                );
            });
        });
        setting_toggle(
            ui,
            "默认双拼",
            "启动后默认使用双拼解析输入。",
            &mut model.default_double_pinyin,
        );
        setting_toggle(
            ui,
            "简拼",
            "允许用声母组合快速输入长词。",
            &mut model.jianpin,
        );
        setting_toggle(
            ui,
            "混拼",
            "允许全拼和声母混合输入词语，默认使用保守匹配。",
            &mut model.mixed_pinyin,
        );
        ui.add_enabled_ui(model.mixed_pinyin, |ui| {
            setting_toggle(
                ui,
                "宽松混拼",
                "放开低可信混拼路径和更多短输入展开，可能增加重码。",
                &mut model.mixed_pinyin_aggressive,
            );
        });
        setting_toggle(
            ui,
            "启用纠错",
            "键盘邻键、漏键等常见拼写错误会参与候选排序。",
            &mut model.correction_enabled,
        );
    });

    ui.add_space(10.0);
    section_panel(ui, "输入辅助", |ui| {
        setting_toggle(
            ui,
            "日期自动格式化",
            "rq、sj 等快捷输入始终使用实时日期时间。",
            &mut model.date_auto_format,
        );
        setting_toggle(
            ui,
            "失败时重试",
            "引擎忙或宿主拒绝编辑时尝试延迟刷新。",
            &mut model.retry_on_failure,
        );
    });
}

pub(super) fn input_tools_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    quiet_section(ui, "辅助输入", |ui| {
        tool_row(
            ui,
            "U",
            "U 模式拆字",
            "使用部件编码辅助输入不认识读音的汉字。",
            |ui| {
                capsule_switch(ui, &mut app.model.u_mode);
            },
        );
        tool_row(
            ui,
            "EN",
            "英语单词输入",
            "中文模式下输入英文前缀时，启用内置 2 万词常用英文候选。",
            |ui| {
                capsule_switch(ui, &mut app.model.english_word_input);
            },
        );
    });
}

pub(super) fn custom_shortcuts_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "自定义短语 / 快捷码", |ui| {
        let response = ui.add(
            TextEdit::multiline(&mut app.model.custom_shortcuts)
                .font(TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .desired_rows(5)
                .hint_text(";qq = name@example.com\n;;r = 此致，敬礼"),
        );
        setting_row_help(response, "每行一个：快捷码 = 候选文本。多个候选用 | 分隔；换行可写成 \\n。保存后直接输入 ;qq 或 ;;r 即可出候选，也可输入 vv ;qq 调出同一组候选。");
    });
}
