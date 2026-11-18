//! Settings page controls.
use super::*;

#[derive(Clone, Copy)]
enum GameTestWizardAction {
    ApplyRecommended,
    ActivateGame,
    LearnInputField,
    InputReady,
    InputMissing,
    CandidateVisible,
    CandidateMissing,
    CandidatePosition,
    PositionGood,
    SavePosition,
    AdjustPosition,
    CommitGood,
    ExitGood,
    UseUnicode,
    UseClipboard,
    Close,
}

pub(super) fn game_test_wizard_ui(ctx: &egui::Context, app: &mut SettingsApp) {
    let Some(wizard) = app.game_test_wizard.clone() else {
        return;
    };
    let mut open = true;
    let mut action = None;
    let dialog_bounds = ctx.screen_rect().shrink(12.0);
    egui::Window::new("游戏中文输入兼容向导")
        .collapsible(false)
        .resizable(false)
        .default_width(520.0_f32.min(dialog_bounds.width()))
        .max_width(dialog_bounds.width())
        .max_height(dialog_bounds.height())
        .constrain_to(dialog_bounds)
        .scroll([false, true])
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            let palette = fluent_palette(ui);
            ui.label(
                RichText::new(&wizard.process)
                    .strong()
                    .size(SETTINGS_FONT_SECTION_TITLE),
            );
            if !wizard.title.is_empty() {
                ui.label(RichText::new(&wizard.title).small().color(palette.muted));
            }
            ui.add_space(8.0);

            match wizard.step {
                GameTestStep::Prepare => {
                    ui.label("第一步：应用游戏推荐配置并保存。游戏按键默认直通；检测到可编辑聊天框后自动启用中文，无法确认时可用中文聊天热键。新游戏先测试标准 TSF 上屏，已保存的方式保持；自动选择 Overlay 后端。");
                    if let Some(rule) = app.model.compat_rules.iter_mut().find(|r| r.process.eq_ignore_ascii_case(&wizard.process)) {
                        game_test_metadata_ui(ui, &mut rule.game_chat);
                    }
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("应用推荐配置并保存").clicked() {
                            action = Some(GameTestWizardAction::ApplyRecommended);
                        }
                        if ui.button("取消").clicked() {
                            action = Some(GameTestWizardAction::Close);
                        }
                    });
                }
                GameTestStep::InputActivation => {
                    if let Some(rule) = app.model.compat_rules.iter_mut().find(|r| r.process.eq_ignore_ascii_case(&wizard.process)) {
                        game_test_metadata_ui(ui, &mut rule.game_chat);
                    }
                    ui.label("第二步：切回游戏并打开聊天输入栏。确认自动进入中文、可以输入拼音，关闭输入栏后移动键恢复直通；无法识别的自绘输入栏可用中文聊天热键。");
                    ui.label("学习输入栏时只保存窗口类、层级和样式特征。探测使用位置、焦点和可写性。");
                    if let Some((line, hint)) = recent_game_input_event(&wizard.process) {
                        ui.label(format!("该进程最近的诊断：{hint}"));
                        ui.label(RichText::new(line).monospace().small().color(palette.muted));
                        ui.label("日志可能来自上一次游戏会话；请以当前游戏里的实际表现为准。");
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("切换到游戏窗口").clicked() { action = Some(GameTestWizardAction::ActivateGame); }
                        if ui.button("学习此游戏的输入栏").clicked() { action = Some(GameTestWizardAction::LearnInputField); }
                        if ui.button("已进入中文输入").clicked() { action = Some(GameTestWizardAction::InputReady); }
                        if ui.button("无法启用中文").clicked() { action = Some(GameTestWizardAction::InputMissing); }
                    });
                }
                GameTestStep::CandidateVisibility => {
                    let backend = matching_compat_rule(&app.model.compat_rules, &wizard.process)
                        .map(|rule| overlay_backend_label(&rule.overlay_backend))
                        .unwrap_or("自动（全屏时独立）");
                    ui.label("第三步：在输入栏键入“nihao”，观察候选栏是否出现。游戏可能自行绘制候选；请确认只有一个候选栏。");
                    ui.label(
                        RichText::new(format!("当前 Overlay 后端：{backend}"))
                            .small()
                            .color(palette.muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("切换到游戏窗口").clicked() {
                            action = Some(GameTestWizardAction::ActivateGame);
                        }
                        if ui.button("能看到候选").clicked() {
                            action = Some(GameTestWizardAction::CandidateVisible);
                        }
                        if ui.button("看不到候选，尝试独立 Overlay").clicked() {
                            action = Some(GameTestWizardAction::CandidateMissing);
                        }
                        if ui.button("候选位置不合适，立即调整").clicked() {
                            action = Some(GameTestWizardAction::CandidatePosition);
                        }
                    });
                    ui.label(
                        RichText::new("真独占全屏可能阻止普通候选窗显示；优先使用无边框全屏。")
                            .small()
                            .color(palette.muted),
                    );
                }
                GameTestStep::Position => {
                    let summary = matching_compat_rule(&app.model.compat_rules, &wizard.process)
                        .map(|rule| {
                            format!(
                                "{}，偏移 ({}, {})，缩放 {}%，{}",
                                overlay_anchor_label(&rule.overlay_anchor),
                                rule.overlay_offset_x,
                                rule.overlay_offset_y,
                                rule.overlay_scale_percent,
                                overlay_monitor_label(&rule.overlay_monitor)
                            )
                        })
                        .unwrap_or_else(|| "自动位置".to_string());
                    ui.label("第四步：确认候选栏的位置、大小和目标显示器是否合适；自动位置优先跟随可信输入位置。");
                    ui.label(RichText::new(summary).small().color(palette.muted));
                    if let Some(rule) = app.model.compat_rules.iter_mut().find(|r| r.process.eq_ignore_ascii_case(&wizard.process)) {
                        ui.label("拖动下方示意候选栏调整相对偏移；保存后请回游戏确认实际位置。");
                        let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().min(440.0), 120.0), egui::Sense::drag());
                        let bounds = response.rect;
                        let painter = painter.with_clip_rect(bounds);
                        painter.rect_filled(bounds, 3.0, egui::Color32::from_rgb(35, 42, 53));
                        let origin = egui::pos2(bounds.left() + 16.0, bounds.bottom() - 32.0);
                        let offset = egui::vec2(rule.overlay_offset_x as f32 * 0.1, rule.overlay_offset_y as f32 * 0.1);
                        let pos = origin + offset;
                        let candidate = egui::Rect::from_min_size(pos, egui::vec2(140.0, 24.0));
                        painter.rect_filled(candidate, 3.0, egui::Color32::from_rgb(45, 104, 190));
                        painter.text(candidate.center(), egui::Align2::CENTER_CENTER, "1 你好   2 拟好", egui::FontId::proportional(13.0), egui::Color32::WHITE);
                        if response.dragged() {
                            let delta = response.drag_delta();
                            rule.overlay_offset_x = (rule.overlay_offset_x + (delta.x * 10.0).round() as i32).clamp(-4000, 4000);
                            rule.overlay_offset_y = (rule.overlay_offset_y + (delta.y * 10.0).round() as i32).clamp(-4000, 4000);
                        }
                        ui.horizontal(|ui| {
                            ui.label("偏移 X / Y");
                            ui.add(egui::DragValue::new(&mut rule.overlay_offset_x).range(-4000..=4000));
                            ui.add(egui::DragValue::new(&mut rule.overlay_offset_y).range(-4000..=4000));
                            ui.label("缩放");
                            ui.add(egui::DragValue::new(&mut rule.overlay_scale_percent).range(50..=200).suffix("%"));
                        });
                    }
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("位置合适").clicked() {
                            action = Some(GameTestWizardAction::PositionGood);
                        }
                        if ui.button("保存位置并回游戏测试").clicked() {
                            action = Some(GameTestWizardAction::SavePosition);
                        }
                        if ui.button("返回规则页调整位置").clicked() {
                            action = Some(GameTestWizardAction::AdjustPosition);
                        }
                    });
                }
                GameTestStep::Commit => {
                    ui.label("第五步：选择候选上屏，亲自确认输入栏出现中文且只出现一次。系统接受事件不代表游戏已经收到文字。");
                    ui.label("请在游戏聊天框输入 nihao，选“你好”，确认没有重复；测试时不要发送聊天消息。");
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("中文上屏一次，继续").clicked() {
                            action = Some(GameTestWizardAction::CommitGood);
                        }
                        if ui.button("尝试 Unicode SendInput").clicked() {
                            action = Some(GameTestWizardAction::UseUnicode);
                        }
                        if ui.button("尝试剪贴板粘贴").clicked() {
                            action = Some(GameTestWizardAction::UseClipboard);
                        }
                    });
                }
                GameTestStep::ExitAndRestore => {
                    ui.label("第六步：有拼音时 Enter 选词、Esc 取消拼音；无拼音时发送或关闭输入栏。检查移动、Shift、数字键恢复正常，切换窗口后没有残留候选或重复文字。");
                    ui.label("若游戏发送后保留输入栏，请把“空 Enter 后”设为“保留中文聊天”或“根据输入栏状态判断”。");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("切换到游戏窗口").clicked() { action = Some(GameTestWizardAction::ActivateGame); }
                        if ui.button("退出与操作恢复正常，保存结果").clicked() { action = Some(GameTestWizardAction::ExitGood); }
                        if ui.button("返回设置调整").clicked() { action = Some(GameTestWizardAction::AdjustPosition); }
                    });
                }
            }
        });

    if !open {
        app.game_test_wizard = None;
        return;
    }
    match action {
        Some(GameTestWizardAction::ApplyRecommended) => {
            app.set_game_profile_for_process(&wizard.process);
            if let Some(rule) = app
                .model
                .compat_rules
                .iter_mut()
                .find(|r| r.process.eq_ignore_ascii_case(&wizard.process))
            {
                rule.game_chat.reset_test_results();
            }
            if app.save().is_ok() {
                if let Some(state) = app.game_test_wizard.as_mut() {
                    state.step = GameTestStep::InputActivation;
                }
            }
        }
        Some(GameTestWizardAction::ActivateGame) => {
            match activate_process_window(&wizard.process) {
                Ok(()) => {
                    app.status = format!(
                        "已切换到 {}，请打开聊天框并键入 nihao；若未自动启用中文，可用游戏聊天快捷键。",
                        wizard.process
                    );
                }
                Err(err) => app.status = err,
            }
        }
        Some(GameTestWizardAction::LearnInputField) => {
            match arm_game_field_learning(&wizard.process)
                .and_then(|()| activate_process_window(&wizard.process))
            {
                Ok(()) => {
                    app.status =
                        "请在 2 分钟内打开聊天框，再按中文聊天热键记录输入栏。".to_string();
                }
                Err(err) => app.status = err,
            }
        }
        Some(GameTestWizardAction::InputReady) => {
            if let Some(rule) = app
                .model
                .compat_rules
                .iter_mut()
                .find(|r| r.process.eq_ignore_ascii_case(&wizard.process))
            {
                if rule.game_chat.tested_display_mode == "unverified" {
                    app.status = "请先选择本次验证的游戏显示模式。窗口大小不能自动证明独占全屏。"
                        .to_string();
                    return;
                }
                rule.game_chat.tested_input = true;
            }
            if let Some(state) = app.game_test_wizard.as_mut() {
                state.step = GameTestStep::CandidateVisibility;
            }
        }
        Some(GameTestWizardAction::InputMissing) => {
            let detail = recent_game_input_event(&wizard.process)
                .map(|(_, hint)| format!("最近诊断：{hint}"))
                .unwrap_or_else(|| "没有该进程的近期输入事件记录；可能未激活输入法，也可能游戏没有向 TSF 提供按键或输入上下文，单凭日志无法确定。".to_string());
            app.status = format!("尚未确认按键接入。请先选择开心输入法并打开游戏输入栏。{detail} 候选显示和上屏方式不能修复按键接入。");
        }
        Some(GameTestWizardAction::CandidateVisible) => {
            if let Some(rule) = app
                .model
                .compat_rules
                .iter_mut()
                .find(|r| r.process.eq_ignore_ascii_case(&wizard.process))
            {
                rule.game_chat.tested_candidate = true;
            }
            if let Some(state) = app.game_test_wizard.as_mut() {
                state.step = GameTestStep::Position;
            }
        }
        Some(GameTestWizardAction::CandidateMissing) => {
            app.set_overlay_backend_for_process(&wizard.process, "external");
            if let Some(rule) = app
                .model
                .compat_rules
                .iter_mut()
                .find(|r| r.process.eq_ignore_ascii_case(&wizard.process))
            {
                rule.game_chat.force_ui = true;
                rule.game_chat.tested_candidate = false;
            }
            if app.save().is_ok() {
                app.status = format!(
                    "已为 {} 改用独立 Overlay；切回游戏后再键入 nihao 测试。",
                    wizard.process
                );
            }
        }
        Some(GameTestWizardAction::CandidatePosition) => {
            if let Some(state) = app.game_test_wizard.as_mut() {
                state.step = GameTestStep::Position;
            }
        }
        Some(GameTestWizardAction::PositionGood) => {
            if let Some(state) = app.game_test_wizard.as_mut() {
                state.step = GameTestStep::Commit;
            }
        }
        Some(GameTestWizardAction::SavePosition) => {
            if app.save().is_ok() {
                if let Some(state) = app.game_test_wizard.as_mut() {
                    state.step = GameTestStep::CandidateVisibility;
                }
                app.status = format!(
                    "已保存 {} 的候选位置，请回游戏确认显示效果。",
                    wizard.process
                );
            }
        }
        Some(GameTestWizardAction::AdjustPosition) => {
            app.active_section = SettingsSection::Compatibility;
            app.reset_section_scroll = true;
            app.game_test_wizard = None;
            app.status = format!(
                "请在 {} 的“候选 Overlay”中调整位置后重新测试。",
                wizard.process
            );
        }
        Some(GameTestWizardAction::CommitGood) => {
            if let Some(rule) = app
                .model
                .compat_rules
                .iter_mut()
                .find(|r| r.process.eq_ignore_ascii_case(&wizard.process))
            {
                rule.game_chat.tested_commit = true;
            }
            if let Some(state) = app.game_test_wizard.as_mut() {
                state.step = GameTestStep::ExitAndRestore;
            }
        }
        Some(GameTestWizardAction::ExitGood) => {
            if let Some(rule) = app
                .model
                .compat_rules
                .iter_mut()
                .find(|r| r.process.eq_ignore_ascii_case(&wizard.process))
            {
                rule.game_chat.tested_exit = true;
            }
            if app.save().is_ok() {
                app.game_test_wizard = None;
                app.status = format!(
                    "已保存 {} 的人工确认结果；仅适用于记录的游戏版本和显示模式。",
                    wizard.process
                );
            }
        }
        Some(GameTestWizardAction::UseUnicode) => {
            app.set_commit_transport_for_process(&wizard.process, "unicode_sendinput");
            if app.save().is_ok() {
                app.status = format!(
                    "已为 {} 改用 Unicode SendInput，请回到游戏重试。",
                    wizard.process
                );
            }
        }
        Some(GameTestWizardAction::UseClipboard) => {
            app.set_commit_transport_for_process(&wizard.process, "clipboard_paste");
            if app.save().is_ok() {
                app.status = format!("已为 {} 改用剪贴板粘贴，请回到游戏重试。", wizard.process);
            }
        }
        Some(GameTestWizardAction::Close) => app.game_test_wizard = None,
        None => {}
    }
}

pub(super) fn game_test_metadata_ui(ui: &mut egui::Ui, chat: &mut GameChatOptions) {
    let before = (
        chat.tested_display_mode.clone(),
        chat.tested_game_version.clone(),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label("验证显示模式")
            .on_hover_text("记录实际人工验证时的游戏显示模式；修改后会清除旧的验证结果。");
        ComboBox::from_id_salt("game_test_display_mode")
            .selected_text(match chat.tested_display_mode.as_str() {
                "windowed" => "窗口",
                "borderless" => "无边框全屏",
                "exclusive" => "独占全屏",
                _ => "尚未验证",
            })
            .show_ui(ui, |ui| {
                for (value, label) in [
                    ("unverified", "尚未验证"),
                    ("windowed", "窗口"),
                    ("borderless", "无边框全屏"),
                    ("exclusive", "独占全屏"),
                ] {
                    ui.selectable_value(&mut chat.tested_display_mode, value.to_string(), label);
                }
            });
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("游戏版本")
            .on_hover_text("填写实际人工验证时的游戏版本；修改后会清除旧的验证结果。");
        ui.add(
            TextEdit::singleline(&mut chat.tested_game_version)
                .char_limit(80)
                .hint_text("可选，手动填写"),
        );
    });
    if before
        != (
            chat.tested_display_mode.clone(),
            chat.tested_game_version.clone(),
        )
    {
        chat.reset_test_results();
    }
}

pub(super) fn game_chat_options_ui(
    ui: &mut egui::Ui,
    chat: &mut GameChatOptions,
    per_app: bool,
    id: &str,
) {
    let before = chat.clone();
    ui.push_id(id, |ui| {
        setting_row(ui, "空 Enter 后", "有拼音时始终先选词。", |ui| {
            ComboBox::from_id_salt("game_enter_behavior").selected_text(match chat.enter_behavior.as_str() {
                "close" => "返回游戏直通", "stay" => "保留中文聊天", "inherit" => "使用全局设置", _ => "根据输入栏状态判断",
            }).show_ui(ui, |ui| {
                if per_app { ui.selectable_value(&mut chat.enter_behavior, "inherit".to_string(), "使用全局设置"); }
                for (value, label) in [("auto", "根据输入栏状态判断"), ("close", "返回游戏直通"), ("stay", "保留中文聊天")] {
                    ui.selectable_value(&mut chat.enter_behavior, value.to_string(), label);
                }
            });
        });
        for (key, label, value) in [("game_auto_uia", "辅助识别输入栏", &mut chat.auto_uia),
            ("game_status_indicator", "中文聊天状态提示", &mut chat.status_indicator)] {
            let description = if key == "game_auto_uia" {
                "通过应用提供的可访问性接口辅助判断输入栏是否可编辑；无法确认时使用手动聊天快捷键。"
            } else {
                "进入中文聊天时显示状态提示；遵守应用隐藏输入法界面的要求。"
            };
            setting_row(ui, label, description, |ui| {
                ComboBox::from_id_salt(key).selected_text(match value.as_str() { "0" => "关闭", "inherit" => "使用全局设置", _ => "开启" })
                    .show_ui(ui, |ui| {
                        if per_app { ui.selectable_value(value, "inherit".to_string(), "使用全局设置"); }
                        ui.selectable_value(value, "1".to_string(), "开启");
                        ui.selectable_value(value, "0".to_string(), "关闭");
                    });
            });
        }
        if !per_app {
            setting_row(ui, "聊天提示时长", "控制游戏聊天状态提示的显示时间。", |ui| {
                ui.add(egui::Slider::new(&mut chat.status_duration_ms, 500..=5000).suffix(" 毫秒"));
            });
            setting_row(ui, "聊天提示大小", "按比例调整游戏聊天状态提示，不改变候选栏大小。", |ui| {
                ui.add(egui::Slider::new(&mut chat.status_scale_percent, 70..=200).suffix("%"));
            });
        }
        if per_app {
            setting_row(ui, "游戏聊天开启键", "例如 Enter、T 或 Y；off 关闭。按键交给游戏，确认输入栏后才开启中文。", |ui| {
                ui.add(TextEdit::singleline(&mut chat.open_key).char_limit(48));
            });
            setting_row(ui, "游戏聊天关闭键", "默认 Escape；有拼音时先取消拼音。", |ui| {
                ui.add(TextEdit::singleline(&mut chat.close_key).char_limit(48));
            });
            setting_toggle(ui, "强制显示输入法候选", "覆盖游戏的隐藏要求。仅在游戏没有自行绘制候选时开启；独立候选窗不能保证覆盖独占全屏。状态提示尊重 UI-less 宿主。", &mut chat.force_ui);
        }
    });
    if before.open_key != chat.open_key || before.auto_uia != chat.auto_uia {
        chat.reset_test_results();
    } else {
        if before.force_ui != chat.force_ui {
            chat.tested_candidate = false;
        }
        if before.close_key != chat.close_key || before.enter_behavior != chat.enter_behavior {
            chat.tested_exit = false;
        }
    }
}

pub(super) fn game_input_mode_label(mode: &str) -> &'static str {
    match mode {
        "passthrough" => "完全直通（英文游戏）",
        "chinese" => "中文输入（始终启用）",
        "auto_text" => "自动识别可编辑输入栏",
        "inherit" => "使用全局游戏策略",
        _ => "默认直通，手动中文聊天",
    }
}

pub(super) fn game_input_mode_combo(
    ui: &mut egui::Ui,
    value: &mut String,
    id: impl std::hash::Hash,
    per_app: bool,
) {
    *value = normalize_game_input_mode(value, per_app);
    ComboBox::from_id_salt(id)
        .selected_text(game_input_mode_label(value))
        .width(bounded_control_width(ui, SETTINGS_CONTROL_WIDTH))
        .wrap()
        .show_ui(ui, |ui| {
            if per_app {
                ui.selectable_value(
                    value,
                    "inherit".to_string(),
                    game_input_mode_label("inherit"),
                );
            }
            for mode in ["manual", "passthrough", "chinese", "auto_text"] {
                ui.selectable_value(value, mode.to_string(), game_input_mode_label(mode));
            }
        });
}

pub(super) fn compatibility_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    section_panel(ui, "游戏与全屏", |ui| {
        compatibility_game_controls_ui(ui, &mut app.model);
    });
    section_panel(ui, "应用规则", |ui| {
        compatibility_rules_ui(ui, app);
        egui::CollapsingHeader::new("从最近运行的应用添加规则")
            .id_salt("compat_recent_processes")
            .show(ui, |ui| recent_process_suggestions_ui(ui, app));
    });
    section_panel(ui, "运行状态", |ui| {
        compatibility_match_status_ui(ui, app);
        egui::CollapsingHeader::new("游戏诊断与上屏预设")
            .id_salt("game_input_summary")
            .show(ui, |ui| game_input_summary_ui(ui, app));
        egui::CollapsingHeader::new("最近兼容 / 降级日志")
            .id_salt("compat_recent_logs")
            .show(ui, compatibility_recent_logs_ui);
    });
}

#[cfg(windows)]
fn arm_game_field_learning(process: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_QWORD,
    };
    if process.is_empty()
        || process.encode_utf16().count() > 128
        || process.contains(['\\', '/', '*', '?', ':', '\0'])
    {
        return Err("请选择具体的游戏进程。".to_string());
    }
    let path: Vec<u16> = format!("Software\\kaixin\\GameChatFields\\{process}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name: Vec<u16> = "LearnUntil".encode_utf16().chain(Some(0)).collect();
    let until = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs()
        + 120)
        .to_le_bytes();
    let mut key = 0;
    let open = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if open != 0 {
        return Err(format!("无法启动输入栏学习（{open}）。"));
    }
    let write = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_QWORD,
            until.as_ptr(),
            until.len() as u32,
        )
    };
    unsafe { RegCloseKey(key) };
    if write != 0 {
        return Err(format!("无法启动输入栏学习（{write}）。"));
    }
    Ok(())
}

#[cfg(not(windows))]
fn arm_game_field_learning(_process: &str) -> Result<(), String> {
    Err("游戏输入栏学习需要 Windows。".to_string())
}

#[derive(Clone, Default)]
struct GameInputSummary {
    confirmed: usize,
    missed: usize,
    tsf_written: usize,
    tsf_failed: usize,
    sent: usize,
    source: String,
    display: String,
}

fn read_game_input_summaries() -> BTreeMap<String, GameInputSummary> {
    let mut summaries = BTreeMap::<String, GameInputSummary>::new();
    for line in runtime_log::recent_lines_matching(400, &["game-chat.session"]) {
        let field = |key: &str| {
            line.split_whitespace()
                .filter_map(|word| word.split_once('='))
                .find_map(|(name, value)| (name == key).then_some(value))
                .unwrap_or_default()
        };
        let process = field("app");
        if process.is_empty() {
            continue;
        }
        let summary = summaries.entry(process.to_ascii_lowercase()).or_default();
        let event = field("event");
        match event {
            "open_confirmed" => {
                summary.confirmed += 1;
                summary.source = field("source").to_string();
            }
            "open_missed" => summary.missed += 1,
            "commit_written" if field("channel") == "tsf" => summary.tsf_written += 1,
            "commit_failed" | "commit_partial" if field("channel") == "tsf" => {
                summary.tsf_failed += 1;
            }
            "commit_sent" => summary.sent += 1,
            _ => {}
        }
        summary.display = field("display").to_string();
    }
    summaries
}

fn game_input_summary_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let id = ui.id().with("game_input_summaries");
    let refresh = ui.button("刷新诊断").clicked();
    let cached: Option<BTreeMap<String, GameInputSummary>> = ui.data(|data| data.get_temp(id));
    let summaries = match cached {
        Some(summaries) if !refresh => summaries,
        _ => {
            let summaries = read_game_input_summaries();
            ui.data_mut(|data| data.insert_temp(id, summaries.clone()));
            summaries
        }
    };
    if summaries.is_empty() {
        ui.label("暂无游戏诊断记录。开启基础日志后，打开聊天栏并上屏一次即可查看。");
        return;
    }
    let mut presets = Vec::new();
    for (process, summary) in &summaries {
        let reliable_tsf = summary.tsf_written >= 3 && summary.tsf_failed == 0;
        let display = if summary.display == "fullscreen_like" {
            "全屏窗口"
        } else {
            "窗口"
        };
        let source = match summary.source.as_str() {
            "native" => "原生编辑框",
            "tsf_scope" => "TSF",
            "uia" => "UI Automation",
            "learned" => "已学习特征",
            "geometry" => "焦点与光标",
            _ => "待确认",
        };
        ui.label(format!(
            "{process}（{display}）　自动识别 {}/{}　来源 {source}　TSF 写入 {} 次　发送未确认 {} 次",
            summary.confirmed,
            summary.confirmed + summary.missed,
            summary.tsf_written,
            summary.sent
        ));
        if reliable_tsf {
            ui.label("建议通道：TSF");
            if matching_compat_rule(&app.model.compat_rules, process).is_some_and(|rule| {
                rule.process.eq_ignore_ascii_case(process)
                    && matches!(rule.commit_transport.as_str(), "auto" | "global")
            }) {
                presets.push(process.clone());
            }
        }
    }
    if !presets.is_empty() && ui.button("保存已验证的 TSF 预设").clicked() {
        for process in presets {
            app.set_commit_transport_for_process(&process, "tsf");
        }
        if app.save().is_ok() {
            app.status = "已保存诊断中验证过的 TSF 上屏预设。".to_string();
        }
    }
}

pub(super) fn compatibility_game_controls_ui(ui: &mut egui::Ui, model: &mut SettingsModel) {
    game_chat_options_ui(ui, &mut model.game_chat, false, "global_game_chat");
    setting_row(
        ui,
        "游戏输入策略",
        "新配置默认自动识别可编辑聊天框：输入栏可写时启用中文，关闭后恢复游戏按键直通。有拼音时 Enter 选词、Esc 取消；无拼音时按设置处理。完全直通关闭所有输入法与工具快捷键。无法确认的自绘输入栏可用手动热键；已有明确设置保持原样。",
        |ui| {
            game_input_mode_combo(
                ui,
                &mut model.game_input_mode,
                "global_game_input_mode",
                false,
            );
        },
    );
    setting_row(
        ui,
        "游戏中文聊天快捷键",
        "在游戏中切换中文聊天与键盘直通；完全直通和始终中文模式不使用此快捷键。",
        |ui| {
            hotkey_combo(
                ui,
                "",
                "compat_game_chat_hotkey",
                &mut model.game_mode_hotkey,
                "G",
            );
        },
    );
    setting_toggle(
        ui,
        "检测全屏",
        "在游戏或全屏应用中自动应用兼容策略。",
        &mut model.fullscreen_detection,
    );
    setting_combo_row(
        ui,
        "全屏策略",
        "进入全屏时如何处理输入法状态和界面。",
        fullscreen_policy_label(&model.fullscreen_policy),
        "fullscreen_policy",
        |ui| {
            selectable_string(
                ui,
                &mut model.fullscreen_policy,
                schema_options::FULLSCREEN_POLICIES[0],
                "显示候选栏",
            );
            selectable_string(
                ui,
                &mut model.fullscreen_policy,
                schema_options::FULLSCREEN_POLICIES[1],
                "英文模式",
            );
            selectable_string(
                ui,
                &mut model.fullscreen_policy,
                schema_options::FULLSCREEN_POLICIES[2],
                "隐藏界面",
            );
            selectable_string(
                ui,
                &mut model.fullscreen_policy,
                schema_options::FULLSCREEN_POLICIES[3],
                "关闭",
            );
        },
    );
    setting_combo_row(
        ui,
        "上屏方式",
        "自动模式先用标准 TSF；游戏中一次上屏只用一种方式，失败时请用测试向导选择并保存。",
        commit_transport_label(&model.commit_transport),
        "commit_transport",
        |ui| {
            selectable_string(
                ui,
                &mut model.commit_transport,
                schema_options::COMMIT_TRANSPORTS[0],
                "自动",
            );
            selectable_string(
                ui,
                &mut model.commit_transport,
                schema_options::COMMIT_TRANSPORTS[1],
                "标准 TSF",
            );
            selectable_string(
                ui,
                &mut model.commit_transport,
                schema_options::COMMIT_TRANSPORTS[2],
                "剪贴板粘贴",
            );
            selectable_string(
                ui,
                &mut model.commit_transport,
                schema_options::COMMIT_TRANSPORTS[3],
                "Unicode SendInput",
            );
        },
    );
    setting_toggle(
        ui,
        "内置游戏进程列表",
        "自动识别常见游戏和全屏应用。",
        &mut model.builtin_game_list,
    );
    setting_toggle(
        ui,
        "自动建议应用规则",
        "根据运行中的应用建议兼容性配置。",
        &mut model.auto_suggest_app_options,
    );
}

pub(super) fn compatibility_match_status_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("当前前台应用")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        if outline_button(ui, "刷新").clicked() {
            app.foreground_process = current_foreground_process();
            app.refresh_recent_processes();
        }
    });
    let Some(foreground) = app.foreground_process.clone() else {
        inline_notice(
            ui,
            StatusTone::Info,
            "未获取到前台窗口。可点击“刷新”重新检测。",
        );
        return;
    };
    let title = if foreground.title.trim().is_empty() {
        "无窗口标题".to_string()
    } else {
        foreground.title.clone()
    };
    let match_text = matching_compat_rule(&app.model.compat_rules, &foreground.name)
        .map(|rule| {
            if rule.enabled {
                let mut text = rule.policy.label().to_string();
                let transport = normalize_commit_transport_value(&rule.commit_transport, "global");
                if transport != "global" {
                    text.push_str(" / ");
                    text.push_str(commit_transport_label(&transport));
                }
                if rule.game_profile {
                    text.push_str(" / 游戏档");
                }
                text
            } else {
                "规则已停用".to_string()
            }
        })
        .unwrap_or_else(|| "未命中自定义规则".to_string());
    ui.label(format!("{}  -  {}", foreground.name, title));
    ui.label(
        RichText::new(format!("命中规则：{match_text}"))
            .small()
            .color(palette.muted),
    );
    let recommended = recommended_compat_policy_for_process(&foreground);
    let likely_game = is_likely_game_process(&foreground);
    let recommended_text = if likely_game {
        "默认键盘直通 / 手动中文聊天 / 紧凑候选栏与自动位置 / 标准 TSF 优先".to_string()
    } else {
        recommended.label().to_string()
    };
    ui.label(
        RichText::new(format!("推荐策略：{recommended_text}"))
            .small()
            .color(palette.muted),
    );
    ui.horizontal_wrapped(|ui| {
        if outline_button(ui, "隐藏候选").clicked() {
            app.set_compat_policy_for_process(&foreground.name, CompatRulePolicy::HideUi);
        }
        if outline_button(ui, "强制英文").clicked() {
            app.set_compat_policy_for_process(&foreground.name, CompatRulePolicy::Ascii);
        }
        if outline_button(ui, "显示候选").clicked() {
            app.set_compat_policy_for_process(&foreground.name, CompatRulePolicy::ShowUi);
        }
        if outline_button(ui, "游戏档").clicked() {
            app.set_game_profile_for_process(&foreground.name);
        }
        if outline_button(ui, "应用推荐").clicked() {
            if likely_game {
                app.set_game_profile_for_process(&foreground.name);
            } else {
                app.set_compat_policy_for_process(&foreground.name, recommended);
            }
        }
        if outline_button(ui, "测试该游戏").clicked() {
            app.open_game_test_wizard(&foreground.name, &foreground.title);
        }
    });
}

pub(crate) fn is_likely_game_process(process: &ProcessSuggestion) -> bool {
    let name = process.name.to_ascii_lowercase();
    let title = process.title.to_ascii_lowercase();
    process.fullscreen
        || name.contains("game")
        || name.contains("steam")
        || name.contains("unity")
        || name.contains("unreal")
        || title.contains("fullscreen")
}

pub(crate) fn recommended_compat_policy_for_process(
    process: &ProcessSuggestion,
) -> CompatRulePolicy {
    if is_likely_game_process(process) {
        CompatRulePolicy::ShowUi
    } else {
        CompatRulePolicy::Global
    }
}

pub(super) fn compatibility_recent_logs_ui(ui: &mut egui::Ui) {
    let palette = fluent_palette(ui);
    ui.label(
        RichText::new("最近兼容/降级日志")
            .strong()
            .size(SETTINGS_FONT_SECTION_TITLE),
    );
    let lines = recent_compatibility_log_lines(8);
    if lines.is_empty() {
        ui.label(
            RichText::new("暂无兼容降级记录。")
                .small()
                .color(palette.muted),
        );
        return;
    }
    egui::Frame::none()
        .fill(palette.surface_alt)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .rounding(SETTINGS_RADIUS_CARD)
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            for line in lines {
                if let Some(hint) = game_input_event_hint(&line) {
                    ui.label(hint);
                }
                ui.label(RichText::new(line).monospace().small().color(palette.text));
            }
        });
}

pub(super) fn compatibility_rules_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("游戏进程匹配列表")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        if outline_button(ui, "添加空规则").clicked() {
            app.model.compat_rules.push(CompatRule {
                enabled: true,
                process: String::new(),
                policy: CompatRulePolicy::ShowUi,
                commit_transport: "auto".to_string(),
                game_input_mode: "auto_text".to_string(),
                game_chat: GameChatOptions::inherited(),
                game_profile: true,
                overlay_anchor: "bottom_left".to_string(),
                overlay_offset_x: 0,
                overlay_offset_y: 0,
                overlay_scale_percent: schema_default::OVERLAY_SCALE_PERCENT,
                overlay_monitor: schema_default::OVERLAY_MONITOR.to_string(),
                overlay_backend: schema_default::OVERLAY_BACKEND.to_string(),
            });
        }
        if let Some(foreground) = app.foreground_process.clone() {
            if outline_button(ui, "添加当前应用").clicked() {
                app.add_compat_process(&foreground.name);
            }
        }
    });

    if app.model.compat_rules.is_empty() {
        ui.label(
            RichText::new("尚未添加自定义兼容规则。")
                .small()
                .color(palette.muted),
        );
        return;
    }

    let mut remove_index = None;
    let mut test_process = None;
    let mut edit_index = None;
    egui::Grid::new("compat_rules_grid")
        .num_columns(6)
        .striped(true)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.strong("启用");
            ui.strong("进程名");
            ui.strong("策略");
            ui.strong("上屏方式");
            ui.strong("游戏档");
            ui.strong("操作");
            ui.end_row();

            for (idx, rule) in app.model.compat_rules.iter_mut().enumerate() {
                ui.checkbox(&mut rule.enabled, "");
                ui.add(TextEdit::singleline(&mut rule.process).desired_width(180.0));
                ComboBox::from_id_salt(("compat_policy", idx))
                    .selected_text(rule.policy.label())
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for policy in CompatRulePolicy::ALL {
                            ui.selectable_value(&mut rule.policy, policy, policy.label());
                        }
                    });
                rule.commit_transport =
                    normalize_commit_transport_value(&rule.commit_transport, "global");
                ComboBox::from_id_salt(("compat_commit_transport", idx))
                    .selected_text(commit_transport_label(&rule.commit_transport))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut rule.commit_transport,
                            "global".to_string(),
                            "使用全局方式",
                        );
                        for transport in schema_options::COMMIT_TRANSPORTS {
                            ui.selectable_value(
                                &mut rule.commit_transport,
                                (*transport).to_string(),
                                commit_transport_label(transport),
                            );
                        }
                    });
                ui.checkbox(&mut rule.game_profile, "");
                ui.horizontal(|ui| {
                    if ui.button("编辑").clicked() {
                        edit_index = Some(idx);
                    }
                    if ui
                        .add_enabled(!rule.process.trim().is_empty(), egui::Button::new("测试"))
                        .clicked()
                    {
                        test_process = Some(rule.process.clone());
                    }
                    if danger_button(ui, "删除").clicked() {
                        remove_index = Some(idx);
                    }
                });
                ui.end_row();
            }
        });

    if let Some(idx) = edit_index {
        app.compat_selected_rule = idx;
    }

    if let Some(idx) = remove_index {
        app.model.compat_rules.remove(idx);
        app.compat_selected_rule = app
            .compat_selected_rule
            .min(app.model.compat_rules.len().saturating_sub(1));
    }
    if !app.model.compat_rules.is_empty() {
        ui.add_space(10.0);
        ui.label(
            RichText::new("每游戏候选 Overlay")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        ui.label(
            RichText::new(
                "位置偏移使用逻辑像素；自动后端会优先选择适合游戏的 Overlay，并在失败时回退。",
            )
            .small()
            .color(palette.muted),
        );

        app.compat_selected_rule = app
            .compat_selected_rule
            .min(app.model.compat_rules.len().saturating_sub(1));
        let selected_rule_label = app
            .model
            .compat_rules
            .get(app.compat_selected_rule)
            .map(|rule| {
                if rule.process.trim().is_empty() {
                    format!("未命名规则 {}", app.compat_selected_rule + 1)
                } else {
                    rule.process.trim().to_string()
                }
            })
            .unwrap_or_else(|| "选择应用".to_string());
        ComboBox::from_id_salt("compat_overlay_selected_rule")
            .selected_text(selected_rule_label)
            .width(260.0)
            .show_ui(ui, |ui| {
                for (idx, rule) in app.model.compat_rules.iter().enumerate() {
                    let label = if rule.process.trim().is_empty() {
                        format!("未命名规则 {}", idx + 1)
                    } else {
                        rule.process.trim().to_string()
                    };
                    ui.selectable_value(&mut app.compat_selected_rule, idx, label);
                }
            });
        ui.add_space(6.0);

        if let Some((idx, rule)) = app
            .model
            .compat_rules
            .iter_mut()
            .enumerate()
            .nth(app.compat_selected_rule)
        {
            let process_label = if rule.process.trim().is_empty() {
                format!("未命名规则 {}", idx + 1)
            } else {
                rule.process.trim().to_string()
            };
            ui.label("该游戏输入策略");
            game_input_mode_combo(
                ui,
                &mut rule.game_input_mode,
                ("app_game_input_mode", idx),
                true,
            );
            egui::CollapsingHeader::new("聊天开启、退出与显示协商")
                .id_salt(("game_chat_options", idx))
                .show(ui, |ui| {
                    game_chat_options_ui(
                        ui,
                        &mut rule.game_chat,
                        true,
                        &format!("game_chat_{idx}"),
                    );
                });
            egui::CollapsingHeader::new("兼容验证记录")
                .id_salt(("game_test_records", idx))
                .show(ui, |ui| {
                    game_test_metadata_ui(ui, &mut rule.game_chat);
                    ui.label(format!(
                        "按键接入：{}；候选：{}；上屏：{}；退出：{}",
                        if rule.game_chat.tested_input {
                            "已确认"
                        } else {
                            "未确认"
                        },
                        if rule.game_chat.tested_candidate {
                            "已确认"
                        } else {
                            "未确认"
                        },
                        if rule.game_chat.tested_commit {
                            "已确认"
                        } else {
                            "未确认"
                        },
                        if rule.game_chat.tested_exit {
                            "已确认"
                        } else {
                            "未确认"
                        }
                    ));
                    if ui.button("清除验证结果").clicked() {
                        rule.game_chat.reset_test_results();
                    }
                });
            rule.overlay_anchor = normalize_overlay_anchor_value(&rule.overlay_anchor);
            rule.overlay_monitor = normalize_overlay_monitor_value(&rule.overlay_monitor);
            rule.overlay_backend = normalize_overlay_backend_value(&rule.overlay_backend);
            rule.overlay_offset_x = rule.overlay_offset_x.clamp(-4000, 4000);
            rule.overlay_offset_y = rule.overlay_offset_y.clamp(-4000, 4000);
            rule.overlay_scale_percent = rule.overlay_scale_percent.clamp(50, 200);
            let overlay_customized = rule.overlay_anchor != schema_default::OVERLAY_ANCHOR
                || rule.overlay_offset_x != 0
                || rule.overlay_offset_y != 0
                || rule.overlay_scale_percent != schema_default::OVERLAY_SCALE_PERCENT
                || rule.overlay_monitor != schema_default::OVERLAY_MONITOR
                || rule.overlay_backend != schema_default::OVERLAY_BACKEND;

            status_badge(
                ui,
                if overlay_customized {
                    StatusTone::Warning
                } else {
                    StatusTone::Success
                },
                if overlay_customized {
                    "Overlay 已自定义"
                } else {
                    "Overlay 推荐值"
                },
            );

            egui::CollapsingHeader::new(format!(
                "{} · {} · {}%",
                process_label,
                overlay_anchor_label(&rule.overlay_anchor),
                rule.overlay_scale_percent
            ))
            .id_salt(("compat_overlay", idx))
            .show(ui, |ui| {
                egui::Grid::new(("compat_overlay_grid", idx))
                    .num_columns(2)
                    .spacing([18.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("锚点").on_hover_text("选择独立候选窗相对于游戏窗口的定位基准。");
                        ComboBox::from_id_salt(("compat_overlay_anchor", idx))
                            .selected_text(overlay_anchor_label(&rule.overlay_anchor))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for anchor in schema_options::OVERLAY_ANCHORS {
                                    ui.selectable_value(
                                        &mut rule.overlay_anchor,
                                        (*anchor).to_string(),
                                        overlay_anchor_label(anchor),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("位置偏移 X / Y").on_hover_text("以像素调整候选窗相对于锚点的水平和垂直位置。");
                        ui.horizontal(|ui| {
                            ui.add(
                                Slider::new(&mut rule.overlay_offset_x, -4000..=4000)
                                    .suffix(" px")
                                    .text("X"),
                            );
                            ui.add(
                                Slider::new(&mut rule.overlay_offset_y, -4000..=4000)
                                    .suffix(" px")
                                    .text("Y"),
                            );
                        });
                        ui.end_row();

                        ui.label("缩放").on_hover_text("单独调整该应用的候选窗大小，范围为 50% 到 200%。");
                        ui.add(Slider::new(&mut rule.overlay_scale_percent, 50..=200).suffix("%"));
                        ui.end_row();

                        ui.label("目标显示器").on_hover_text("自动跟随游戏所在显示器，也可指定主显示器或显示器编号。");
                        ComboBox::from_id_salt(("compat_overlay_monitor", idx))
                            .selected_text(overlay_monitor_label(&rule.overlay_monitor))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut rule.overlay_monitor,
                                    "auto".to_string(),
                                    "游戏所在显示器（推荐）",
                                );
                                ui.selectable_value(
                                    &mut rule.overlay_monitor,
                                    "primary".to_string(),
                                    "主显示器",
                                );
                                for monitor_index in 0..8 {
                                    ui.selectable_value(
                                        &mut rule.overlay_monitor,
                                        monitor_index.to_string(),
                                        format!("显示器 {}", monitor_index + 1),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("Overlay 后端").on_hover_text("自动选择候选后端，并尊重游戏接管候选绘制；只有启用强制显示才覆盖宿主的隐藏要求。独占全屏可能仍无法显示外部候选。");
                        ComboBox::from_id_salt(("compat_overlay_backend", idx))
                            .selected_text(overlay_backend_label(&rule.overlay_backend))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for backend in schema_options::OVERLAY_BACKENDS {
                                    ui.selectable_value(
                                        &mut rule.overlay_backend,
                                        (*backend).to_string(),
                                        overlay_backend_label(backend),
                                    );
                                }
                            });
                        ui.end_row();
                    });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("恢复游戏推荐").clicked() {
                        rule.overlay_anchor = schema_default::OVERLAY_ANCHOR.to_string();
                        rule.overlay_offset_x = 0;
                        rule.overlay_offset_y = 0;
                        rule.overlay_scale_percent = schema_default::OVERLAY_SCALE_PERCENT;
                        rule.overlay_monitor = schema_default::OVERLAY_MONITOR.to_string();
                        rule.overlay_backend = schema_default::OVERLAY_BACKEND.to_string();
                    }
                    if ui
                        .add_enabled(
                            !rule.process.trim().is_empty(),
                            egui::Button::new("测试该游戏"),
                        )
                        .clicked()
                    {
                        test_process = Some(rule.process.clone());
                    }
                });
            });
        }
    }
    sync_compat_rules_to_legacy_fields(&mut app.model);
    if let Some(process) = test_process {
        let title = app
            .recent_processes
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&process))
            .map(|item| item.title.clone())
            .unwrap_or_default();
        app.open_game_test_wizard(&process, &title);
    }
}

pub(super) fn recent_process_suggestions_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let palette = fluent_palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("最近进程推荐")
                .strong()
                .size(SETTINGS_FONT_SECTION_TITLE),
        );
        if outline_button(ui, "刷新").clicked() {
            app.refresh_recent_processes();
        }
    });
    ui.horizontal_wrapped(|ui| {
        for process in app.recent_processes.clone() {
            let mut label = format!("+ {}", process.name);
            if process.foreground {
                label.push_str(" 前台");
            }
            if process.fullscreen {
                label.push_str(" 全屏");
            }
            let response = ui.small_button(label);
            let response = if process.title.trim().is_empty() {
                response
            } else {
                response.on_hover_text(process.title.clone())
            };
            if response.clicked() {
                app.add_compat_process(&process.name);
            }
        }
    });
    ui.label(
        RichText::new("优先显示前台窗口和带标题的应用，已过滤系统后台进程。")
            .small()
            .color(palette.muted),
    );
}
