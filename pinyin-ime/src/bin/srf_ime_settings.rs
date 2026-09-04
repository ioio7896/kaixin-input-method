#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "srf_ime_settings/config.rs"]
mod config;
#[path = "../fonts.rs"]
mod fonts;
#[path = "srf_ime_settings/model.rs"]
mod model;
#[path = "srf_ime_settings/ui.rs"]
mod ui;

use config::*;
use model::*;
use ui::{
    capsule_switch, diagnostic_log_paths, enforce_settings_min_font_size,
    export_diagnostic_package_to, fluent_palette, privacy_statement_text,
};

use eframe::egui::{
    self, Color32, ComboBox, FontId, RichText, Slider, Stroke, TextEdit, TextStyle,
};
use pinyin_ime::app_paths;
use pinyin_ime::config_schema::{
    default as schema_default, key as schema_key, options as schema_options,
    section as schema_section,
};
use pinyin_ime::external_translation;
use pinyin_ime::rapidocr_paths;
use pinyin_ime::runtime_log;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(windows)]
use std::ffi::OsString;
use std::fs;
use std::io::Write;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const WINDOW_TITLE: &str = "开心输入法 设置";

const TITLE_CN: &str = "输入法设置";
const SAVE_CN: &str = "保存设置";
const RESET_CN: &str = "重置默认";
const OPEN_CFG_CN: &str = "打开配置目录";
const SAVED_CN: &str = "已保存";
const LOAD_FAIL_CN: &str = "读取配置失败";
const SAVE_FAIL_CN: &str = "保存失败";
const USER_DICT_RELOAD_HINT: &str =
    "用户词库数据变更后，新引擎会自动读取；当前正在输入的宿主可能需要切换一次输入法。";
const HANDWRITE_EXE: &str = "srf_ime_handwrite.exe";
const OCR_EXE: &str = "srf_ime_ocr.exe";
const CANDIDATE_OVERLAY_EXE: &str = "srf_ime_overlay.exe";
const SETTINGS_WINDOW_SIZE: [f32; 2] = [1000.0, 940.0];
const SETTINGS_MAX_WINDOW_SIZE: [f32; 2] = [1160.0, 1040.0];
const SETTINGS_MIN_WINDOW_SIZE: [f32; 2] = [780.0, 680.0];
const SETTINGS_NAV_WIDTH: f32 = 212.0;
const SETTINGS_PANEL_RADIUS: f32 = 10.0;
const SETTINGS_USER_DICT_LIST_LIMIT: usize = 100;
const MS_PINYIN_TIP: &str =
    r"0804:{81D4E9C9-1D3B-41BC-9E6C-4B40BF79E35E}{FA550B04-5AD7-411F-A5AC-CA038EC515D7}";
const US_KEYBOARD_TIP: &str = r"0409:00000409";
const KAIXIN_TIP: &str =
    r"0804:{E5A91C40-7B2D-4F8A-9C11-8F3E6D2A1B00}{A3F0B2C1-4D5E-6789-ABCD-EF0123456789}";
const VV_COMMAND_HELP: &[(&str, &str, &str)] = &[
    ("vv rq/date/jr", "日期", "vv rq"),
    ("vv sj/time", "时间", "vv sj"),
    ("vv xq/week/zhou", "星期", "vv xq"),
    ("vv sym/fh", "符号", "vv sym"),
    ("vv emoji/emjio/face", "Emoji", "vv emoji smile"),
    ("vv unit/dw", "单位", "vv unit"),
    ("vv dx/rmb/money", "金额大写", "vv rmb 123.45"),
    ("vv mail/email", "邮箱片段", "vv mail"),
    ("vv url/site/http", "网址片段", "vv url"),
    ("vv md/markdown", "Markdown", "vv md"),
    ("vv cb/clip/paste", "剪贴板候选", "vv cb"),
    ("vv hw/handwrite/sx", "手写查字", "vv hw"),
    ("vvu", "剪贴板管理器", "vvu"),
];

struct SettingsApp {
    model: SettingsModel,
    config: IniDoc,
    config_path: PathBuf,
    status: String,
    save_toast: Option<(String, Instant)>,
    last_saved_model: SettingsModel,
    confirm_close_with_unsaved_changes: bool,
    user_phrase_key: String,
    user_phrase_text: String,
    blocked_phrase_text: String,
    blocked_phrases: Vec<pinyin_ime::user_dict::ManagedBlockedPhrase>,
    blocked_phrases_loaded: bool,
    user_dict_task_rx: Option<mpsc::Receiver<UserDictTaskResult>>,
    available_skins: Vec<SkinPreview>,
    available_chinese_fonts: Vec<String>,
    recent_processes: Vec<ProcessSuggestion>,
    foreground_process: Option<ProcessSuggestion>,
    game_test_wizard: Option<GameTestWizard>,
    active_section: SettingsSection,
    reset_section_scroll: bool,
    vv_command_filter: String,
    diagnostics_cache: Option<ui::DiagnosticsSnapshot>,
}

enum UserDictTaskResult {
    BlockPhrase {
        phrase: String,
        result: Result<bool, String>,
    },
    UnblockPhrase {
        phrase: String,
        result: Result<bool, String>,
    },
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let _ = fonts::install_cjk_fonts(&cc.egui_ctx);
        enforce_settings_min_font_size(&cc.egui_ctx);
        let config_path = app_paths::config_ini_path()
            .unwrap_or_else(|| PathBuf::from(app_paths::CONFIG_FILE_NAME));

        let available_skins = discover_skin_previews();
        let available_chinese_fonts = fonts::installed_chinese_font_families();
        let recent_processes = recent_process_suggestions(32);
        let foreground_process = current_foreground_process();
        let (config, mut model, status) = match load_config(&config_path) {
            Ok((config, model)) => (config, model, String::new()),
            Err(err) => (
                IniDoc::default(),
                SettingsModel::default(),
                format!("{LOAD_FAIL_CN}: {err}"),
            ),
        };
        merge_discovered_lexicon_tags_compat(&mut model);
        let last_saved_model = model.clone();

        Self {
            model,
            config,
            config_path,
            status,
            save_toast: None,
            last_saved_model,
            confirm_close_with_unsaved_changes: false,
            user_phrase_key: String::new(),
            user_phrase_text: String::new(),
            blocked_phrase_text: String::new(),
            blocked_phrases: Vec::new(),
            blocked_phrases_loaded: false,
            user_dict_task_rx: None,
            available_skins,
            available_chinese_fonts,
            recent_processes,
            foreground_process,
            game_test_wizard: None,
            active_section: SettingsSection::Hotkeys,
            reset_section_scroll: false,
            vv_command_filter: String::new(),
            diagnostics_cache: None,
        }
    }

    fn reset_defaults(&mut self) {
        self.replace_model_preserving_lexicons(SettingsModel::default());
        self.status.clear();
    }

    fn replace_model_preserving_lexicons(&mut self, mut model: SettingsModel) {
        let tags = self
            .model
            .lexicon_tags
            .keys()
            .map(|key| {
                (
                    key.clone(),
                    pinyin_ime::lexicon_prefs::default_optional_lexicon_tag_enabled(key),
                )
            })
            .collect();
        model.lexicon_tags = tags;
        self.model = model;
        sync_compat_rules_to_legacy_fields(&mut self.model);
    }

    fn reset_rank_defaults(&mut self) {
        let default = SettingsModel::default();
        self.model.w_single_lm = default.w_single_lm;
        self.model.w_phrase_path = default.w_phrase_path;
        self.model.lm_single_scale = default.lm_single_scale;
        self.status = "排序权重已恢复推荐值。".to_string();
    }

    fn reset_engine_tuning_defaults(&mut self) {
        let default = SettingsModel::default();
        self.model.prefix_cache_capacity = default.prefix_cache_capacity;
        self.model.final_lookup_cache_capacity = default.final_lookup_cache_capacity;
        self.model.short_lookup_cache_capacity = default.short_lookup_cache_capacity;
        self.model.long_lookup_soft_budget_ms = default.long_lookup_soft_budget_ms;
        self.model.long_lookup_min_first_batch_candidates =
            default.long_lookup_min_first_batch_candidates;
        self.status = "引擎性能参数已恢复推荐值。".to_string();
    }

    fn set_compat_policy_for_process(&mut self, process: &str, policy: CompatRulePolicy) {
        let normalized = process.trim();
        if normalized.is_empty() {
            return;
        }
        let commit_transport = self
            .model
            .compat_rules
            .iter()
            .find(|rule| rule.process.eq_ignore_ascii_case(normalized))
            .map(|rule| rule.commit_transport.clone())
            .unwrap_or_else(|| "global".to_string());
        let game_profile = self
            .model
            .compat_rules
            .iter()
            .find(|rule| rule.process.eq_ignore_ascii_case(normalized))
            .map(|rule| rule.game_profile)
            .unwrap_or(false);
        upsert_compat_rule(
            &mut self.model.compat_rules,
            normalized,
            true,
            policy,
            &commit_transport,
            game_profile,
        );
        sync_compat_rules_to_legacy_fields(&mut self.model);
        self.status = format!("已为 {normalized} 设置兼容策略：{}", policy.label());
    }

    fn set_game_profile_for_process(&mut self, process: &str) {
        let normalized = process.trim();
        if normalized.is_empty() {
            return;
        }
        upsert_compat_rule(
            &mut self.model.compat_rules,
            normalized,
            true,
            CompatRulePolicy::ShowUi,
            "tsf",
            true,
        );
        if let Some(rule) = self
            .model
            .compat_rules
            .iter_mut()
            .find(|rule| rule.process.eq_ignore_ascii_case(normalized))
        {
            rule.overlay_backend = schema_default::OVERLAY_BACKEND.to_string();
        }
        sync_compat_rules_to_legacy_fields(&mut self.model);
        self.status = format!("已为 {normalized} 启用游戏配置档（先测试标准 TSF 上屏）。");
    }

    fn set_overlay_backend_for_process(&mut self, process: &str, backend: &str) {
        let normalized = normalized_process_name(process);
        if let Some(rule) = self
            .model
            .compat_rules
            .iter_mut()
            .find(|rule| rule.process.eq_ignore_ascii_case(&normalized))
        {
            rule.overlay_backend = normalize_overlay_backend_value(backend);
            sync_compat_rules_to_legacy_fields(&mut self.model);
        }
    }

    fn set_commit_transport_for_process(&mut self, process: &str, transport: &str) {
        let normalized = normalized_process_name(process);
        if let Some(rule) = self
            .model
            .compat_rules
            .iter_mut()
            .find(|rule| rule.process.eq_ignore_ascii_case(&normalized))
        {
            rule.commit_transport = normalize_commit_transport_value(transport, "auto");
            sync_compat_rules_to_legacy_fields(&mut self.model);
        }
    }

    fn open_game_test_wizard(&mut self, process: &str, title: &str) {
        let process = normalized_process_name(process);
        if process.is_empty() {
            self.status = "请先填写要测试的游戏进程名。".to_string();
            return;
        }
        self.game_test_wizard = Some(GameTestWizard {
            process,
            title: title.trim().to_string(),
            step: GameTestStep::Prepare,
        });
    }

    fn save(&mut self) -> Result<(), String> {
        let mut next_model = self.model.clone();
        sync_compat_rules_to_legacy_fields(&mut next_model);
        normalize_combo_values(&mut next_model);

        let conflicts = hotkey_conflicts(&next_model);
        if !conflicts.is_empty() {
            let message = format!("保存已取消：快捷键冲突：{}", conflicts.join(" / "));
            self.status = message.clone();
            return Err(message);
        }

        let previous_model = model_from_config(&self.config);
        let rendered = stabilized_rendered_config(&self.config, &next_model);
        if let Some(parent) = self.config_path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                self.status = format!("{SAVE_FAIL_CN}: {err}");
                return Err(self.status.clone());
            }
        }
        match write_config_atomically(&self.config_path, &rendered) {
            Ok(()) => {
                self.config = parse_ini(&rendered);
                self.model = model_from_config(&self.config);
                merge_discovered_lexicon_tags_compat(&mut self.model);
                self.last_saved_model = self.model.clone();
                let effect_summary = save_effect_summary(&previous_model, &self.model);
                self.status = format!(
                    "{SAVED_CN}: {}。{}",
                    self.config_path.display(),
                    effect_summary
                );
                self.save_toast = Some((self.status.clone(), Instant::now()));
                Ok(())
            }
            Err(err) => {
                self.status = format!("{SAVE_FAIL_CN}: {err}");
                Err(self.status.clone())
            }
        }
    }

    fn open_config_dir(&mut self) {
        let dir = self
            .config_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        if let Err(err) = open_path(&dir) {
            self.status = err;
        }
    }

    fn refresh_recent_processes(&mut self) {
        self.recent_processes = recent_process_suggestions(32);
        self.foreground_process = current_foreground_process();
        self.status = format!("已刷新最近进程：{} 个。", self.recent_processes.len());
    }

    fn add_compat_process(&mut self, process: &str) {
        let normalized = process.trim();
        if normalized.is_empty() {
            return;
        }
        if add_compat_rule_unique(
            &mut self.model.compat_rules,
            normalized,
            CompatRulePolicy::ShowUi,
        ) {
            if let Some(rule) = self
                .model
                .compat_rules
                .iter_mut()
                .find(|rule| rule.process.eq_ignore_ascii_case(normalized))
            {
                rule.commit_transport = "auto".to_string();
                rule.game_profile = true;
            }
            sync_compat_rules_to_legacy_fields(&mut self.model);
            self.status = format!("已加入游戏配置档：{normalized}");
        } else {
            self.status = format!("兼容进程已存在：{normalized}");
        }
    }

    fn export_user_dict(&mut self) {
        let confirmed = matches!(
            rfd::MessageDialog::new()
                .set_title("导出用户词库")
                .set_description(
                    "导出的用户词库 SQLite 是明文数据库，包含已学习的词、编码、词频和上下文排序信号。请只保存到可信位置。"
                )
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show(),
            rfd::MessageDialogResult::Ok
        );
        if !confirmed {
            self.status = "已取消导出用户词库。".to_string();
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .set_title("导出用户词库")
            .set_file_name("user_dict_export.sqlite")
            .save_file()
        else {
            return;
        };
        match pinyin_ime::user_dict::export_user_dict(&path) {
            Ok(()) => self.status = format!("已导出明文用户词库：{}", path.display()),
            Err(err) => self.status = format!("导出用户词库失败: {err}"),
        }
    }

    fn export_decrypted_user_dict(&mut self) {
        let confirmed = matches!(
            rfd::MessageDialog::new()
                .set_title("解密导出用户词库")
                .set_description(
                    "将把本机加密的用户词库导出为明文 SQLite，包含已学习的词、编码、词频和上下文排序信号。请只保存到可信位置。"
                )
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show(),
            rfd::MessageDialogResult::Ok
        );
        if !confirmed {
            self.status = "已取消解密导出用户词库。".to_string();
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .set_title("解密导出用户词库")
            .set_file_name("user_dict_decrypted.sqlite")
            .save_file()
        else {
            return;
        };
        match pinyin_ime::user_dict::export_decrypted_user_dict(&path) {
            Ok(()) => self.status = format!("已解密导出用户词库：{}", path.display()),
            Err(err) => self.status = format!("解密导出用户词库失败: {err}"),
        }
    }

    fn export_user_dict_tsv(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("导出便携用户词表")
            .set_file_name("kaixin_user_words.tsv")
            .add_filter("TSV", &["tsv"])
            .save_file()
        else {
            return;
        };
        match pinyin_ime::user_dict::export_user_dict_tsv(&path) {
            Ok(()) => self.status = format!("已导出便携用户词表：{}", path.display()),
            Err(err) => self.status = format!("导出便携用户词表失败: {err}"),
        }
    }

    fn import_user_dict(&mut self) {
        self.import_user_dict_mode(pinyin_ime::user_dict::UserDictImportMode::Merge);
    }

    fn replace_user_dict(&mut self) {
        self.import_user_dict_mode(pinyin_ime::user_dict::UserDictImportMode::Replace);
    }

    fn import_user_dict_mode(&mut self, mode: pinyin_ime::user_dict::UserDictImportMode) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("导入用户词库")
            .add_filter("用户词库", &["sqlite", "db", "tsv"])
            .pick_file()
        else {
            return;
        };
        let preview = match pinyin_ime::user_dict::preview_user_dict_import(&path) {
            Ok(preview) => preview,
            Err(err) => {
                self.status = format!("读取用户词库失败: {err}");
                return;
            }
        };
        let action = if mode == pinyin_ime::user_dict::UserDictImportMode::Merge {
            "合并"
        } else {
            "完全覆盖"
        };
        let description = format!(
            "准备{action}用户词库。\n\n词条：{}\n新增：{}\n重复：{}\n异读冲突：{}\n置顶：{}\n屏蔽：{}\n上下文记录：{}\n\n导入前会自动备份当前词库；合并模式不会导入上下文和负反馈。",
            preview.total_entries,
            preview.new_entries,
            preview.duplicate_entries,
            preview.reading_conflicts,
            preview.pinned_entries,
            preview.blocked_entries,
            preview.context_entries,
        );
        let confirmed = matches!(
            rfd::MessageDialog::new()
                .set_title("确认导入用户词库")
                .set_description(&description)
                .set_level(
                    if mode == pinyin_ime::user_dict::UserDictImportMode::Replace {
                        rfd::MessageLevel::Warning
                    } else {
                        rfd::MessageLevel::Info
                    }
                )
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show(),
            rfd::MessageDialogResult::Ok
        );
        if !confirmed {
            self.status = "已取消导入用户词库。".to_string();
            return;
        }
        match pinyin_ime::user_dict::import_user_dict_with_mode(&path, mode) {
            Ok(()) => {
                self.blocked_phrases_loaded = false;
                self.status = format!(
                    "已{action}用户词库：新增 {} 条，重复 {} 条，冲突 {} 条。{USER_DICT_RELOAD_HINT}",
                    preview.new_entries, preview.duplicate_entries, preview.reading_conflicts
                );
            }
            Err(err) => self.status = format!("导入用户词库失败: {err}"),
        }
    }

    fn add_user_phrase(&mut self) {
        match pinyin_ime::user_dict::add_user_phrase(&self.user_phrase_key, &self.user_phrase_text)
        {
            Ok(()) => {
                self.user_phrase_key.clear();
                self.user_phrase_text.clear();
                self.blocked_phrases_loaded = false;
                self.status = format!("已添加用户词。{USER_DICT_RELOAD_HINT}");
            }
            Err(err) => self.status = format!("添加用户词失败: {err}"),
        }
    }

    fn load_blocked_phrases(&mut self, show_status: bool) {
        match pinyin_ime::user_dict::list_blocked_phrases(SETTINGS_USER_DICT_LIST_LIMIT) {
            Ok(items) => {
                let count = items.len();
                self.blocked_phrases = items;
                self.blocked_phrases_loaded = true;
                if show_status {
                    self.status = format!("已刷新永远不学名单：{count} 条");
                }
            }
            Err(err) => {
                self.blocked_phrases.clear();
                self.blocked_phrases_loaded = true;
                self.status = format!("读取永远不学名单失败: {err}");
            }
        }
    }

    fn block_phrase_from_settings(&mut self) {
        let phrase = self.blocked_phrase_text.trim().to_string();
        if phrase.is_empty() {
            return;
        }
        if self.user_dict_task_rx.is_some() {
            self.status = "用户词库操作正在处理中，请稍后。".to_string();
            return;
        }
        self.blocked_phrase_text.clear();
        let (tx, rx) = mpsc::channel();
        let task_phrase = phrase.clone();
        thread::spawn(move || {
            let result = pinyin_ime::user_dict::block_user_phrase(&task_phrase)
                .map_err(|err| err.to_string());
            let _ = tx.send(UserDictTaskResult::BlockPhrase {
                phrase: task_phrase,
                result,
            });
        });
        self.user_dict_task_rx = Some(rx);
        self.status = format!("正在加入永远不学：{phrase}");
    }

    fn poll_user_dict_task(&mut self) {
        let Some(rx) = self.user_dict_task_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(UserDictTaskResult::BlockPhrase { phrase, result }) => match result {
                Ok(true) => {
                    self.load_blocked_phrases(false);
                    self.status = format!("已加入永远不学：{phrase}。{USER_DICT_RELOAD_HINT}");
                }
                Ok(false) => self.status = format!("无法加入永远不学：{phrase}"),
                Err(err) => self.status = format!("加入永远不学失败: {err}"),
            },
            Ok(UserDictTaskResult::UnblockPhrase { phrase, result }) => match result {
                Ok(true) => {
                    self.load_blocked_phrases(false);
                    self.status = format!("已移出永远不学：{phrase}。{USER_DICT_RELOAD_HINT}");
                }
                Ok(false) => {
                    self.load_blocked_phrases(false);
                    self.status = format!("未找到永远不学项：{phrase}");
                }
                Err(err) => self.status = format!("移出永远不学失败: {err}"),
            },
            Err(mpsc::TryRecvError::Empty) => {
                self.user_dict_task_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = "用户词库后台操作已中断。".to_string();
            }
        }
    }

    fn unblock_phrase_from_settings(&mut self, phrase: &str) {
        if self.user_dict_task_rx.is_some() {
            self.status = "用户词库操作正在处理中，请稍后。".to_string();
            return;
        }
        let phrase = phrase.to_string();
        let (tx, rx) = mpsc::channel();
        let task_phrase = phrase.clone();
        thread::spawn(move || {
            let result = pinyin_ime::user_dict::unblock_user_phrase(&task_phrase)
                .map_err(|err| err.to_string());
            let _ = tx.send(UserDictTaskResult::UnblockPhrase {
                phrase: task_phrase,
                result,
            });
        });
        self.user_dict_task_rx = Some(rx);
        self.status = format!("正在移出永远不学：{phrase}");
    }

    fn clear_clipboard(&mut self) {
        match pinyin_ime::clipboard_store::clear_all() {
            Ok(()) => self.status = "已清空剪贴板历史和置顶项。".to_string(),
            Err(err) => self.status = format!("清空剪贴板失败: {err}"),
        }
    }

    fn clear_user_dict(&mut self) {
        match pinyin_ime::user_dict::clear_user_dict() {
            Ok(()) => {
                self.blocked_phrases.clear();
                self.blocked_phrases_loaded = true;
                self.status = format!("已清空用户词库。{USER_DICT_RELOAD_HINT}");
            }
            Err(err) => self.status = format!("清空用户词库失败: {err}"),
        }
    }

    fn export_privacy_statement(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("导出隐私说明")
            .set_file_name("kaixin-ime-privacy.txt")
            .save_file()
        else {
            return;
        };
        let text = privacy_statement_text(&self.config_path, &self.model);
        match fs::write(&path, text) {
            Ok(()) => self.status = format!("已导出隐私说明：{}", path.display()),
            Err(err) => self.status = format!("导出隐私说明失败: {err}"),
        }
    }

    fn clear_tsf_log(&mut self) {
        let mut removed = 0usize;
        let mut last_err = None;
        for path in diagnostic_log_paths() {
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => last_err = Some(format!("{}: {err}", path.display())),
            }
        }
        if let Some(err) = last_err {
            self.status = format!("清空日志失败: {err}");
        } else if removed == 0 {
            self.status = "日志文件尚未生成。".to_string();
        } else {
            self.status = format!("已清空 {removed} 个日志文件。");
        }
    }

    fn export_diagnostic_package(&mut self) {
        let Some(base_dir) = rfd::FileDialog::new().set_title("导出诊断包").pick_folder()
        else {
            return;
        };
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let dest = base_dir.join(format!("kaixin-diagnostics-{stamp}"));
        match export_diagnostic_package_to(&dest, &self.config_path, &self.model) {
            Ok(()) => self.status = format!("已导出诊断包：{}", dest.display()),
            Err(err) => self.status = format!("导出诊断包失败: {err}"),
        }
    }

    fn add_microsoft_pinyin(&mut self) {
        self.status = match run_language_list_action(LanguageListAction::AddMicrosoftPinyin) {
            Ok(()) => "已添加中文（中国）微软拼音输入法。".to_string(),
            Err(err) => format!("添加微软拼音失败: {err}"),
        };
    }

    fn remove_microsoft_pinyin(&mut self) {
        self.status = match run_language_list_action(LanguageListAction::RemoveMicrosoftPinyin) {
            Ok(()) => "已删除中文（中国）微软拼音输入法。".to_string(),
            Err(err) => format!("删除微软拼音失败: {err}"),
        };
    }

    fn add_us_keyboard(&mut self) {
        self.status = match run_language_list_action(LanguageListAction::AddUsKeyboard) {
            Ok(()) => "已添加英文（美国）美式键盘。".to_string(),
            Err(err) => format!("添加美式键盘失败: {err}"),
        };
    }

    fn remove_us_keyboard(&mut self) {
        self.status = match run_language_list_action(LanguageListAction::RemoveUsKeyboard) {
            Ok(()) => "已删除英文（美国）美式键盘。".to_string(),
            Err(err) => format!("删除美式键盘失败: {err}"),
        };
    }

    fn pin_kaixin_input(&mut self) {
        self.status = match run_language_list_action(LanguageListAction::PinKaixin) {
            Ok(()) => "已将开心输入法置顶。".to_string(),
            Err(err) => format!("置顶开心输入法失败: {err}"),
        };
    }

    fn unpin_kaixin_input(&mut self) {
        self.status = match run_language_list_action(LanguageListAction::UnpinKaixin) {
            Ok(()) => "已取消开心输入法置顶。".to_string(),
            Err(err) => format!("取消置顶开心输入法失败: {err}"),
        };
    }

    fn open_data_location(&mut self, path: PathBuf) {
        let target = if path.is_dir() {
            path
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        };
        if let Err(err) = open_path(&target) {
            self.status = err;
        }
    }

    fn backup_config(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("备份配置")
            .set_file_name(app_paths::CONFIG_FILE_NAME)
            .save_file()
        else {
            return;
        };
        match fs::copy(&self.config_path, &path) {
            Ok(_) => self.status = format!("已备份配置到 {}", path.display()),
            Err(err) => self.status = format!("备份配置失败: {err}"),
        }
    }

    fn restore_config(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("恢复配置")
            .add_filter("INI", &["ini"])
            .pick_file()
        else {
            return;
        };
        match fs::read_to_string(&path) {
            Ok(text) => {
                self.config = parse_ini(&text);
                self.model = model_from_config(&self.config);
                merge_discovered_lexicon_tags_compat(&mut self.model);
                self.last_saved_model = self.model.clone();
                let _ = self.save();
            }
            Err(err) => self.status = format!("恢复配置失败: {err}"),
        }
    }
    fn open_handwrite(&mut self) {
        let Some(path) = resolve_runtime_exe_path(HANDWRITE_EXE) else {
            self.status = format!("未找到手写查字程序：{HANDWRITE_EXE}");
            return;
        };
        let mut cmd = Command::new(&path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            cmd.current_dir(parent);
        }
        match cmd.spawn() {
            Ok(_) => self.status = "已打开手写查字窗口。".to_string(),
            Err(err) => self.status = format!("无法启动手写查字：{err}"),
        }
    }
    fn open_ocr(&mut self) {
        let Some(path) = resolve_runtime_exe_path(OCR_EXE) else {
            self.status = format!("未找到 OCR 程序：{OCR_EXE}");
            return;
        };
        let mut cmd = Command::new(&path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            cmd.current_dir(parent);
        }
        cmd.arg("--manual-region");
        match cmd.spawn() {
            Ok(_) => self.status = "已打开 OCR 窗口。".to_string(),
            Err(err) => self.status = format!("无法启动 OCR：{err}"),
        }
    }

    fn open_ocr_translate(&mut self) {
        let Some(path) = resolve_runtime_exe_path(OCR_EXE) else {
            self.status = format!("未找到 OCR 程序：{OCR_EXE}");
            return;
        };
        let mut cmd = Command::new(&path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            cmd.current_dir(parent);
        }
        cmd.arg("--manual-region");
        cmd.arg("--translate");
        match cmd.spawn() {
            Ok(_) => self.status = "已打开截图翻译。".to_string(),
            Err(err) => self.status = format!("无法启动截图翻译：{err}"),
        }
    }

    fn open_translate(&mut self) {
        match external_translation::open_translator() {
            Ok(()) => self.status = "已打开独立翻译软件。".to_string(),
            Err(err) => self.status = err,
        }
    }

    fn check_ocr_language(&mut self) {
        self.status = match check_rapidocr_environment() {
            Ok(message) => message,
            Err(err) => format!("RapidOCR 检测失败：{err}"),
        };
    }

    fn check_translation_environment(&mut self) {
        self.status = check_translation_environment();
    }
}

impl SettingsApp {
    fn import_lexicon_text(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("导入开心词库（.txt）")
            .add_filter("开心词库", &["txt"])
            .pick_file()
        else {
            return;
        };
        let Some(root) = pinyin_ime::core::default_phrase_lexicon_dir()
            .or_else(|| app_paths::local_data_dir().map(|dir| dir.join("lexicon")))
        else {
            self.status = "未找到可用词库目录。".to_string();
            return;
        };
        // Imported dictionaries are optional extensions. Keep the curated
        // zh main lexicon immutable from settings and place imports in zh-ext
        // so they receive an explicit on/off switch.
        let dest_dir = root.join("zh-ext");
        if let Err(err) = fs::create_dir_all(&dest_dir) {
            self.status = format!("创建导入词库目录失败: {err}");
            return;
        }
        let dest_name = imported_lexicon_file_name(&path);
        let mut dest = dest_dir.join(&dest_name);
        let mut idx = 2usize;
        while dest.exists() {
            dest = dest_dir.join(format!("{idx}-{dest_name}"));
            idx += 1;
        }
        match fs::copy(&path, &dest) {
            Ok(_) => {
                self.reload_lexicon_now();
                merge_discovered_lexicon_tags_compat(&mut self.model);
                self.status = format!("已导入词库：{}。{}", dest.display(), self.status);
            }
            Err(err) => self.status = format!("导入词库失败: {err}"),
        }
    }

    fn reload_lexicon_now(&mut self) {
        pinyin_ime::lexicon_prefs::request_phrase_lexicon_reload();
        self.status = "已请求引擎重新加载词库。正在输入的应用会在下一次查询时生效。".to_string();
    }
}

fn imported_lexicon_file_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("imported");
    let mut out = String::from("imported_");
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out == "imported_" {
        out.push_str("lexicon");
    }
    out.push_str(".txt");
    out
}

enum LanguageListAction {
    AddMicrosoftPinyin,
    RemoveMicrosoftPinyin,
    AddUsKeyboard,
    RemoveUsKeyboard,
    PinKaixin,
    UnpinKaixin,
}

fn run_language_list_action(action: LanguageListAction) -> Result<(), String> {
    #[cfg(windows)]
    {
        let action_name = match action {
            LanguageListAction::AddMicrosoftPinyin => "add-ms-pinyin",
            LanguageListAction::RemoveMicrosoftPinyin => "remove-ms-pinyin",
            LanguageListAction::AddUsKeyboard => "add-us-keyboard",
            LanguageListAction::RemoveUsKeyboard => "remove-us-keyboard",
            LanguageListAction::PinKaixin => "pin-kaixin",
            LanguageListAction::UnpinKaixin => "unpin-kaixin",
        };
        let script = language_list_script(action_name);
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .output()
            .map_err(|err| err.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("PowerShell exit code {:?}", output.status.code())
            };
            Err(detail)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        Err("此功能仅支持 Windows。".to_string())
    }
}

#[cfg(windows)]
fn language_list_script(action: &str) -> String {
    format!(
        r#"
$ErrorActionPreference = 'Stop'
$action = '{action}'
$msPinyin = '{ms_pinyin}'
$usKeyboard = '{us_keyboard}'
$kaixin = '{kaixin}'
$list = Get-WinUserLanguageList

function Ensure-Language([string]$tag) {{
    foreach ($item in $list) {{
        if ($item.LanguageTag -ieq $tag) {{ return $item }}
    }}
    $newList = New-WinUserLanguageList $tag
    $item = $newList[0]
    [void]$list.Add($item)
    return $item
}}

function Ensure-Tip([string]$tag, [string]$tip) {{
    $item = Ensure-Language $tag
    if (-not ($item.InputMethodTips -contains $tip)) {{
        [void]$item.InputMethodTips.Add($tip)
    }}
}}

function Remove-Tip([string]$tip) {{
    foreach ($item in $list) {{
        while ($item.InputMethodTips -contains $tip) {{
            [void]$item.InputMethodTips.Remove($tip)
        }}
    }}
}}

function Remove-Language-If-Empty([string]$tag) {{
    for ($i = $list.Count - 1; $i -ge 0; $i--) {{
        if ($list[$i].LanguageTag -ieq $tag -and $list[$i].InputMethodTips.Count -eq 0) {{
            $list.RemoveAt($i)
        }}
    }}
}}

function Move-Language-To-Top([string]$tag) {{
    $top = $null
    $rest = New-Object 'System.Collections.Generic.List[Microsoft.InternationalSettings.Commands.WinUserLanguage]'
    foreach ($item in $list) {{
        if ($null -eq $top -and $item.LanguageTag -ieq $tag) {{
            $top = $item
        }} else {{
            [void]$rest.Add($item)
        }}
    }}
    if ($null -ne $top) {{
        $list.Clear()
        [void]$list.Add($top)
        foreach ($item in $rest) {{ [void]$list.Add($item) }}
    }}
}}

switch ($action) {{
    'add-ms-pinyin' {{
        Ensure-Tip 'zh-CN' $msPinyin
    }}
    'remove-ms-pinyin' {{
        Remove-Tip $msPinyin
        Remove-Language-If-Empty 'zh-CN'
    }}
    'add-us-keyboard' {{
        Ensure-Tip 'en-US' $usKeyboard
    }}
    'remove-us-keyboard' {{
        Remove-Tip $usKeyboard
        Remove-Language-If-Empty 'en-US'
    }}
    'pin-kaixin' {{
        Remove-Tip $kaixin
        $zh = Ensure-Language 'zh-CN'
        $zh.InputMethodTips.Insert(0, $kaixin)
        Move-Language-To-Top 'zh-CN'
    }}
    'unpin-kaixin' {{
        Remove-Tip $kaixin
        Ensure-Tip 'zh-CN' $kaixin
    }}
    default {{
        throw "Unknown language-list action: $action"
    }}
}}

Set-WinUserLanguageList $list -Force
"#,
        action = action,
        ms_pinyin = MS_PINYIN_TIP,
        us_keyboard = US_KEYBOARD_TIP,
        kaixin = KAIXIN_TIP
    )
}

fn open_path(path: &PathBuf) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("open folder is only available on Windows".to_string())
    }
}

fn resolve_runtime_exe_path(exe_name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(exe_name));
        }
    }
    if let Ok(dir) = std::env::current_dir() {
        candidates.push(dir.join(exe_name));
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(la)
                .join("Programs")
                .join(app_paths::APP_PATH_NAME)
                .join(exe_name),
        );
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(
            PathBuf::from(pf)
                .join(app_paths::APP_PATH_NAME)
                .join(exe_name),
        );
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn resolve_candidate_overlay_path() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    if let Ok(dir) = std::env::current_dir() {
        roots.push(dir);
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        roots.push(
            PathBuf::from(la)
                .join("Programs")
                .join(app_paths::APP_PATH_NAME),
        );
    }
    if let Ok(pf) = std::env::var("ProgramFiles(x86)") {
        roots.push(PathBuf::from(pf).join(app_paths::APP_PATH_NAME));
    }

    // Keep the real-renderer preview usable from an unpackaged developer build.
    let mut development_roots = Vec::new();
    for root in &roots {
        for ancestor in root.ancestors().take(6) {
            development_roots.push(
                ancestor
                    .join("tsf-tip")
                    .join("build-package")
                    .join("Release"),
            );
        }
    }
    roots.extend(development_roots);

    for root in roots {
        let direct = root.join(CANDIDATE_OVERLAY_EXE);
        if direct.is_file() {
            return Some(direct);
        }
        let manifest = root.join("current_runtime_payload.txt");
        let Ok(relative) = fs::read_to_string(manifest) else {
            continue;
        };
        let relative = relative.trim();
        if relative.is_empty()
            || Path::new(relative).is_absolute()
            || Path::new(relative)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            continue;
        }
        let candidate = root.join(relative).join("x64").join(CANDIDATE_OVERLAY_EXE);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn launch_real_candidate_preview() -> Result<(), String> {
    let path = resolve_candidate_overlay_path()
        .ok_or_else(|| format!("未找到 {CANDIDATE_OVERLAY_EXE}"))?;
    let mut command = Command::new(&path);
    command.arg("--candidate-preview");
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn translation_available() -> bool {
    external_translation::is_available()
}

#[allow(unreachable_code)]
fn check_translation_environment() -> String {
    external_translation::test_connection().unwrap_or_else(|error| {
        format!(
            "{}\n连接测试失败：{error}",
            external_translation::availability_message()
        )
    })
}

fn check_rapidocr_environment() -> Result<String, String> {
    let rapidocr_root =
        rapidocr_paths::rapidocr_root().ok_or_else(rapidocr_paths::missing_root_message)?;
    rapidocr_paths::validate_rapidocr_models(&rapidocr_root)?;
    let rapidocr_python = rapidocr_root.join("python");
    if !rapidocr_python.join("rapidocr").is_dir() {
        return Err(format!(
            "RapidOCR Python 包目录不存在：{}",
            rapidocr_python.display()
        ));
    }

    let script = format!(
        "import sys; sys.path.insert(0, r'{python_dir}'); import rapidocr; import cv2; import numpy; import onnxruntime; print('RapidOCR 可用：{root}')",
        python_dir = rapidocr_python.display(),
        root = rapidocr_root.display(),
    );
    let runtime = rapidocr_paths::python_runtime(Some(&rapidocr_root))?;
    let mut command = Command::new(&runtime.executable);
    rapidocr_paths::apply_python_runtime_env(&mut command, &runtime);
    let output = command
        .args(["-c", &script])
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .map_err(|err| format!("启动 Python 失败：{err}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("Python 返回 {}", output.status)
        } else {
            err
        });
    }
    let message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if message.is_empty() {
        format!("RapidOCR 可用：{}", rapidocr_root.display())
    } else {
        message
    })
}

fn selectable_string(ui: &mut egui::Ui, value: &mut String, option: &str, label: &str) {
    ui.selectable_value(value, option.to_string(), label);
}

fn fixed_letter_hotkey_combo(
    ui: &mut egui::Ui,
    label: &str,
    combo_id: &str,
    value: &mut String,
    default_letter: char,
) {
    hotkey_combo(ui, label, combo_id, value, &default_letter.to_string());
}

fn hotkey_combo(
    ui: &mut egui::Ui,
    label: &str,
    combo_id: &str,
    value: &mut String,
    default_key: &str,
) {
    let mut parts = hotkey_parts(value, default_key);
    let mut enabled = !is_hotkey_disabled(value);
    let available_width = ui.available_width().max(160.0);
    ui.set_max_width(available_width);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(available_width);
        if !label.is_empty() {
            ui.label(label);
        }
        let enabled_changed = capsule_switch(ui, &mut enabled)
            .on_hover_text("启用或关闭此快捷键")
            .changed();
        ui.label(
            RichText::new(if enabled { "已启用" } else { "已关闭" })
                .small()
                .color(fluent_palette(ui).muted),
        );
        let mut changed = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.set_max_width(available_width);
            changed |= ui.checkbox(&mut parts.ctrl, "Ctrl").changed();
            changed |= ui.checkbox(&mut parts.shift, "Shift").changed();
            changed |= ui.checkbox(&mut parts.alt, "Alt").changed();
            if !parts.ctrl && !parts.shift && !parts.alt {
                parts.ctrl = true;
                changed = true;
            }
            ComboBox::from_id_salt(combo_id)
                .selected_text(hotkey_key_label(&parts.key))
                .width(78.0)
                .show_ui(ui, |ui| {
                    for (key, label) in hotkey_key_options() {
                        changed |= ui
                            .selectable_value(&mut parts.key, key.to_string(), label)
                            .changed();
                    }
                });
        });
        if enabled_changed {
            if enabled {
                *value = hotkey_value(parts);
            } else {
                *value = "off".to_string();
            }
        } else if enabled && changed {
            *value = hotkey_value(parts);
        }
    });
}

fn is_hotkey_disabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "off" | "none" | "disabled" | "关闭"
    )
}

fn is_explicit_hotkey_disabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "off" | "none" | "disabled" | "关闭"
    )
}

fn normalized_hotkey_key_value(value: &str, default_key: &str) -> String {
    if is_explicit_hotkey_disabled(value) {
        "off".to_string()
    } else {
        hotkey_value(hotkey_parts(value, default_key))
    }
}

fn parse_fixed_letter_hotkey(value: &str) -> Option<FixedLetterHotkeyParts> {
    let lowered = value.trim().to_ascii_lowercase();
    if is_hotkey_disabled(&lowered) {
        return None;
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut key = None;
    for part in lowered.replace(['_', '-'], "+").split('+').map(str::trim) {
        if part.is_empty() {
            continue;
        }
        match part {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            _ => {
                let normalized = normalize_hotkey_key(part)?;
                if key.replace(normalized).is_some() {
                    return None;
                }
            }
        }
    }

    Some(FixedLetterHotkeyParts {
        ctrl,
        shift,
        alt,
        key: key?,
    })
}

#[derive(Clone)]
struct FixedLetterHotkeyParts {
    ctrl: bool,
    shift: bool,
    alt: bool,
    key: String,
}

fn hotkey_value(parts: FixedLetterHotkeyParts) -> String {
    let mut out = Vec::new();
    if parts.ctrl || (!parts.shift && !parts.alt) {
        out.push("Ctrl".to_string());
    }
    if parts.shift {
        out.push("Shift".to_string());
    }
    if parts.alt {
        out.push("Alt".to_string());
    }
    out.push(parts.key);
    out.join("+")
}

fn normalized_hotkey_value(value: &str, default_key: &str) -> String {
    normalized_hotkey_key_value(value, default_key)
}

fn hotkey_parts(value: &str, default_key: &str) -> FixedLetterHotkeyParts {
    parse_fixed_letter_hotkey(value).unwrap_or(FixedLetterHotkeyParts {
        ctrl: true,
        shift: true,
        alt: true,
        key: normalize_hotkey_key(default_key).unwrap_or_else(|| "A".to_string()),
    })
}

fn hotkey_key_options() -> Vec<(&'static str, &'static str)> {
    let mut options = Vec::new();
    for key in [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    ] {
        options.push((key, key));
    }
    options.extend([
        ("Space", "Space"),
        ("Tab", "Tab"),
        ("Enter", "Enter"),
        ("Esc", "Esc"),
        ("Comma", ","),
        ("Period", "."),
        ("Slash", "/"),
        ("Semicolon", ";"),
        ("Quote", "'"),
        ("Minus", "-"),
        ("Equal", "="),
    ]);
    options
}

fn hotkey_key_label(key: &str) -> String {
    hotkey_key_options()
        .into_iter()
        .find(|(value, _)| value.eq_ignore_ascii_case(key))
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| key.to_string())
}

fn normalize_hotkey_key(key: &str) -> Option<String> {
    let trimmed = key.trim();
    if trimmed.len() == 1 {
        let ch = trimmed.chars().next()?;
        if ch.is_ascii_alphanumeric() {
            return Some(ch.to_ascii_uppercase().to_string());
        }
        return match ch {
            ',' => Some("Comma".to_string()),
            '.' => Some("Period".to_string()),
            '/' => Some("Slash".to_string()),
            ';' => Some("Semicolon".to_string()),
            '\'' => Some("Quote".to_string()),
            '-' => Some("Minus".to_string()),
            '=' => Some("Equal".to_string()),
            _ => None,
        };
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "space" => Some("Space".to_string()),
        "tab" => Some("Tab".to_string()),
        "enter" | "return" => Some("Enter".to_string()),
        "esc" | "escape" => Some("Esc".to_string()),
        "comma" => Some("Comma".to_string()),
        "period" | "dot" => Some("Period".to_string()),
        "slash" => Some("Slash".to_string()),
        "semicolon" => Some("Semicolon".to_string()),
        "quote" | "apostrophe" => Some("Quote".to_string()),
        "minus" => Some("Minus".to_string()),
        "equal" | "equals" => Some("Equal".to_string()),
        _ => None,
    }
}

fn matching_compat_rule<'a>(rules: &'a [CompatRule], process: &str) -> Option<&'a CompatRule> {
    let normalized = normalized_process_name(process);
    rules
        .iter()
        .find(|rule| normalized_process_name(&rule.process).eq_ignore_ascii_case(&normalized))
}

fn recent_process_suggestions(limit: usize) -> Vec<ProcessSuggestion> {
    let mut suggestions = Vec::new();
    let mut seen = BTreeSet::new();
    collect_visible_window_processes(&mut suggestions, &mut seen, limit);
    collect_recent_processes_from_runtime_events(&mut suggestions, &mut seen, limit);
    collect_recent_processes_from_tsf_log(&mut suggestions, &mut seen, limit);
    collect_processes_from_tasklist(&mut suggestions, &mut seen, limit);
    suggestions.sort_by(|a, b| {
        b.foreground
            .cmp(&a.foreground)
            .then_with(|| b.fullscreen.cmp(&a.fullscreen))
            .then_with(|| (!b.title.trim().is_empty()).cmp(&(!a.title.trim().is_empty())))
            .then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
    });
    suggestions.truncate(limit);
    suggestions
}

fn collect_recent_processes_from_runtime_events(
    suggestions: &mut Vec<ProcessSuggestion>,
    seen: &mut BTreeSet<String>,
    limit: usize,
) {
    if suggestions.len() >= limit {
        return;
    }
    for name in runtime_log::recent_process_names(limit.saturating_mul(2).max(limit)) {
        push_process_suggestion(
            suggestions,
            seen,
            ProcessSuggestion {
                name,
                title: String::new(),
                foreground: false,
                fullscreen: false,
            },
            limit,
        );
        if suggestions.len() >= limit {
            return;
        }
    }
}

fn collect_recent_processes_from_tsf_log(
    suggestions: &mut Vec<ProcessSuggestion>,
    seen: &mut BTreeSet<String>,
    limit: usize,
) {
    for path in diagnostic_log_paths() {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines().rev().take(800) {
            for key in ["process=", "app="] {
                if let Some(name) = extract_log_field(line, key) {
                    push_process_suggestion(
                        suggestions,
                        seen,
                        ProcessSuggestion {
                            name,
                            title: String::new(),
                            foreground: false,
                            fullscreen: false,
                        },
                        limit,
                    );
                }
            }
            if suggestions.len() >= limit {
                return;
            }
        }
    }
}

fn collect_processes_from_tasklist(
    suggestions: &mut Vec<ProcessSuggestion>,
    seen: &mut BTreeSet<String>,
    limit: usize,
) {
    if suggestions.len() >= limit {
        return;
    }
    let Ok(output) = hidden_tasklist_output() else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(name) = csv_first_field(line) {
            push_process_suggestion(
                suggestions,
                seen,
                ProcessSuggestion {
                    name,
                    title: String::new(),
                    foreground: false,
                    fullscreen: false,
                },
                limit,
            );
        }
        if suggestions.len() >= limit {
            return;
        }
    }
}

fn hidden_tasklist_output() -> std::io::Result<std::process::Output> {
    let mut command = Command::new("tasklist");
    command.args(["/FO", "CSV", "/NH"]);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.output()
}

fn extract_log_field(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let value = line[start..]
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .trim();
    if value.is_empty() || value == "(unknown)" {
        None
    } else {
        Some(value.to_string())
    }
}

fn csv_first_field(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    } else {
        Some(trimmed.split(',').next()?.trim().to_string())
    }
}

fn push_process_suggestion(
    suggestions: &mut Vec<ProcessSuggestion>,
    seen: &mut BTreeSet<String>,
    mut suggestion: ProcessSuggestion,
    limit: usize,
) {
    if suggestions.len() >= limit {
        return;
    }
    suggestion.name = normalized_process_name(&suggestion.name);
    if !looks_like_process_name(&suggestion.name) {
        return;
    }
    let key = suggestion.name.to_ascii_lowercase();
    if seen.insert(key) {
        suggestions.push(suggestion);
    }
}

fn looks_like_process_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.is_empty()
        && lower.ends_with(".exe")
        && !lower.contains('\\')
        && !lower.contains('/')
        && !matches!(
            lower.as_str(),
            "applicationframehost.exe"
                | "audiodg.exe"
                | "conhost.exe"
                | "csrss.exe"
                | "ctfmon.exe"
                | "dwm.exe"
                | "explorer.exe"
                | "fontdrvhost.exe"
                | "lsass.exe"
                | "memory compression"
                | "registry"
                | "runtimebroker.exe"
                | "searchhost.exe"
                | "securityhealthservice.exe"
                | "services.exe"
                | "sihost.exe"
                | "smss.exe"
                | "spoolsv.exe"
                | "srf_ime_engine.exe"
                | "srf_ime_settings.exe"
                | "srf_ime_tray.exe"
                | "startmenuexperiencehost.exe"
                | "svchost.exe"
                | "system"
                | "system idle process"
                | "taskhostw.exe"
                | "wininit.exe"
                | "winlogon.exe"
        )
}

#[cfg(windows)]
fn collect_visible_window_processes(
    suggestions: &mut Vec<ProcessSuggestion>,
    seen: &mut BTreeSet<String>,
    limit: usize,
) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowPlacement, IsWindowVisible, SW_SHOWMINIMIZED, WINDOWPLACEMENT,
    };

    struct EnumState<'a> {
        suggestions: &'a mut Vec<ProcessSuggestion>,
        seen: &'a mut BTreeSet<String>,
        limit: usize,
        foreground: HWND,
    }

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam as *mut EnumState<'_>);
        if state.suggestions.len() >= state.limit {
            return 0;
        }
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
        placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        if GetWindowPlacement(hwnd, &mut placement) != 0
            && placement.showCmd == SW_SHOWMINIMIZED as u32
        {
            return 1;
        }
        let Some(name) = process_name_for_hwnd(hwnd) else {
            return 1;
        };
        let title = window_title(hwnd);
        if title.trim().is_empty() {
            return 1;
        }
        push_process_suggestion(
            state.suggestions,
            state.seen,
            ProcessSuggestion {
                name,
                title,
                foreground: hwnd == state.foreground,
                fullscreen: is_window_probably_fullscreen(hwnd),
            },
            state.limit,
        );
        1
    }

    let foreground = current_foreground_hwnd();
    let mut state = EnumState {
        suggestions,
        seen,
        limit,
        foreground,
    };
    unsafe {
        EnumWindows(
            Some(enum_windows_proc),
            &mut state as *mut EnumState<'_> as LPARAM,
        );
    }
}

#[cfg(not(windows))]
fn collect_visible_window_processes(
    _suggestions: &mut Vec<ProcessSuggestion>,
    _seen: &mut BTreeSet<String>,
    _limit: usize,
) {
}

#[cfg(windows)]
fn current_foreground_hwnd() -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow() }
}

#[cfg(windows)]
fn current_foreground_process() -> Option<ProcessSuggestion> {
    let hwnd = current_foreground_hwnd();
    if hwnd == 0 {
        return None;
    }
    let name = process_name_for_hwnd(hwnd)?;
    if !looks_like_process_name(&name) {
        return None;
    }
    Some(ProcessSuggestion {
        name,
        title: window_title(hwnd),
        foreground: true,
        fullscreen: is_window_probably_fullscreen(hwnd),
    })
}

#[cfg(windows)]
fn activate_process_window(process: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    struct FindWindowState {
        target: String,
        hwnd: HWND,
    }

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam as *mut FindWindowState);
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        if process_name_for_hwnd(hwnd)
            .is_some_and(|name| normalized_process_name(&name).eq_ignore_ascii_case(&state.target))
        {
            state.hwnd = hwnd;
            return 0;
        }
        1
    }

    let target = normalized_process_name(process)
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(process)
        .to_string();
    let mut state = FindWindowState { target, hwnd: 0 };
    unsafe {
        EnumWindows(
            Some(enum_windows_proc),
            &mut state as *mut FindWindowState as LPARAM,
        );
    }
    if state.hwnd == 0 {
        return Err(format!("未找到正在运行的游戏窗口：{process}"));
    }
    unsafe {
        if IsIconic(state.hwnd) != 0 {
            ShowWindow(state.hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(state.hwnd) == 0 {
            return Err("Windows 暂时不允许切换前台窗口，请手动 Alt+Tab 到游戏。".to_string());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn activate_process_window(_process: &str) -> Result<(), String> {
    Err("切换游戏窗口仅支持 Windows。".to_string())
}

#[cfg(not(windows))]
fn current_foreground_process() -> Option<ProcessSuggestion> {
    None
}

#[cfg(windows)]
fn process_name_for_hwnd(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<String> {
    use pinyin_ime::win_handle::OwnedWinHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    // SAFETY: OpenProcess returned a new process handle owned here.
    let process = unsafe { OwnedWinHandle::from_raw(process) }.ok()?;
    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    let ok =
        unsafe { QueryFullProcessImageNameW(process.as_raw(), 0, buffer.as_mut_ptr(), &mut size) };
    if ok == 0 || size == 0 {
        return None;
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..size as usize]));
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
}

#[cfg(windows)]
fn window_title(hwnd: windows_sys::Win32::Foundation::HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; (len as usize) + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied <= 0 {
        return String::new();
    }
    OsString::from_wide(&buffer[..copied as usize])
        .to_string_lossy()
        .trim()
        .to_string()
}

#[cfg(windows)]
fn is_window_probably_fullscreen(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut rect: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor == 0 {
        return false;
    }
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return false;
    }
    const TOLERANCE: i32 = 8;
    rect.left <= info.rcMonitor.left + TOLERANCE
        && rect.top <= info.rcMonitor.top + TOLERANCE
        && rect.right >= info.rcMonitor.right - TOLERANCE
        && rect.bottom >= info.rcMonitor.bottom - TOLERANCE
}

fn save_effect_summary(before: &SettingsModel, after: &SettingsModel) -> String {
    if before == after {
        return "未发现新的设置差异；配置文件已刷新。".to_string();
    }

    let mut hot_loaded = Vec::new();
    if input_behavior_settings_changed(before, after) {
        hot_loaded.push("输入习惯");
    }
    if candidate_settings_changed(before, after) {
        hot_loaded.push("候选外观");
    }
    if lexicon_learning_settings_changed(before, after) {
        hot_loaded.push("词库/学习");
    }
    if privacy_data_settings_changed(before, after) {
        hot_loaded.push("隐私与数据规则");
    }
    if tool_data_settings_changed(before, after) {
        hot_loaded.push("工具、截图和剪贴板");
    }
    if notification_settings_changed(before, after) {
        hot_loaded.push("通知");
    }
    if advanced_engine_settings_changed(before, after) {
        hot_loaded.push("排序和性能参数");
    }

    let mut parts = Vec::new();
    if !hot_loaded.is_empty() {
        parts.push(format!(
            "已写入并由输入法/托盘热加载：{}。",
            hot_loaded.join("、")
        ));
    }
    if global_hotkey_settings_changed(before, after) {
        parts.push(
            "全局快捷键由托盘重新注册，通常几秒内生效；若被占用会在快捷键页显示失败。".to_string(),
        );
    }
    if compatibility_settings_changed(before, after) {
        parts.push("兼容规则和全屏策略会在应用重新获得焦点时最稳生效。".to_string());
    }
    if tool_window_runtime_settings_changed(before, after) {
        parts.push("已打开的 OCR/翻译窗口需要重开才会读取常驻或结果动作变化。".to_string());
    }
    if parts.is_empty() {
        parts.push("配置文件已刷新。".to_string());
    }
    parts.join("")
}

fn stabilized_rendered_config(config: &IniDoc, model: &SettingsModel) -> String {
    let mut rendered = rendered_config_for_model(config, model);
    for _ in 0..4 {
        let reparsed = parse_ini(&rendered);
        let reparsed_model = model_from_config(&reparsed);
        let next = rendered_config_for_model(&reparsed, &reparsed_model);
        if next == rendered {
            return rendered;
        }
        rendered = next;
    }
    rendered
}

fn input_behavior_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.default_ascii != after.default_ascii
        || before.global_ascii != after.global_ascii
        || before.default_full_shape != after.default_full_shape
        || before.default_chinese_punct != after.default_chinese_punct
        || before.curly_punct != after.curly_punct
        || before.auto_pair_punct != after.auto_pair_punct
        || before.number_fullwidth != after.number_fullwidth
        || before.symbol_fullwidth != after.symbol_fullwidth
        || before.shift_symbol_temporary_ascii != after.shift_symbol_temporary_ascii
        || before.date_auto_format != after.date_auto_format
        || before.english_word_input != after.english_word_input
        || before.traditional_output != after.traditional_output
        || before.default_fuzzy_pinyin != after.default_fuzzy_pinyin
        || before.fuzzy_zh_z != after.fuzzy_zh_z
        || before.fuzzy_ch_c != after.fuzzy_ch_c
        || before.fuzzy_sh_s != after.fuzzy_sh_s
        || before.fuzzy_n_l != after.fuzzy_n_l
        || before.fuzzy_f_h != after.fuzzy_f_h
        || before.fuzzy_an_ang != after.fuzzy_an_ang
        || before.fuzzy_en_eng != after.fuzzy_en_eng
        || before.fuzzy_in_ing != after.fuzzy_in_ing
        || before.default_double_pinyin != after.default_double_pinyin
        || before.double_pinyin_schema != after.double_pinyin_schema
        || before.jianpin != after.jianpin
        || before.mixed_pinyin != after.mixed_pinyin
        || before.mixed_pinyin_aggressive != after.mixed_pinyin_aggressive
        || before.v_assist != after.v_assist
        || before.symbol_toolbox != after.symbol_toolbox
        || before.emoji_input != after.emoji_input
        || before.u_mode != after.u_mode
        || before.custom_shortcuts != after.custom_shortcuts
        || before.show_status_notifications != after.show_status_notifications
        || before.retry_on_failure != after.retry_on_failure
        || before.correction_enabled != after.correction_enabled
        || before.cn_en_hotkey != after.cn_en_hotkey
        || before.full_shape_hotkey != after.full_shape_hotkey
        || before.punct_hotkey != after.punct_hotkey
        || before.fuzzy_hotkey != after.fuzzy_hotkey
        || before.double_pinyin_hotkey != after.double_pinyin_hotkey
        || before.shift_tap_hotkey != after.shift_tap_hotkey
        || before.candidate_number_select != after.candidate_number_select
        || before.candidate_left_click != after.candidate_left_click
        || before.candidate_right_click != after.candidate_right_click
        || before.page_minus_equal != after.page_minus_equal
        || before.page_comma_period != after.page_comma_period
        || before.page_pgup_pgdn != after.page_pgup_pgdn
        || before.traditional_hotkey != after.traditional_hotkey
        || before.game_mode_hotkey != after.game_mode_hotkey
        || before.temporary_ascii_hotkey != after.temporary_ascii_hotkey
}

fn candidate_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.inline_preedit != after.inline_preedit
        || before.enhanced_position != after.enhanced_position
        || before.paging_on_scroll != after.paging_on_scroll
        || before.candidate_page_size != after.candidate_page_size
        || before.candidate_horizontal != after.candidate_horizontal
        || before.candidate_horizontal_count != after.candidate_horizontal_count
        || before.candidate_horizontal_compact != after.candidate_horizontal_compact
        || before.candidate_font_size != after.candidate_font_size
        || before.candidate_opacity != after.candidate_opacity
        || before.candidate_reduce_motion != after.candidate_reduce_motion
        || before.candidate_font_weight != after.candidate_font_weight
        || before.candidate_selected_font_weight != after.candidate_selected_font_weight
        || before.candidate_label_font_weight != after.candidate_label_font_weight
        || before.candidate_chip_font_weight != after.candidate_chip_font_weight
        || before.candidate_font_file != after.candidate_font_file
        || before.candidate_skin_file != after.candidate_skin_file
        || before.theme != after.theme
        || before.candidate_material != after.candidate_material
        || before.candidate_density != after.candidate_density
        || before.candidate_layout_variant != after.candidate_layout_variant
        || before.candidate_vertical_layout_variant != after.candidate_vertical_layout_variant
        || before.candidate_horizontal_layout_variant != after.candidate_horizontal_layout_variant
        || before.candidate_topmost != after.candidate_topmost
        || before.show_candidate_reading != after.show_candidate_reading
        || before.show_candidate_score != after.show_candidate_score
        || before.highlight_typo_candidates != after.highlight_typo_candidates
        || before.show_candidate_source != after.show_candidate_source
        || before.show_mode_in_candidate_header != after.show_mode_in_candidate_header
        || before.candidate_abbreviate_length != after.candidate_abbreviate_length
}

fn lexicon_learning_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.learning_sensitivity != after.learning_sensitivity
        || before.user_hotword_boost != after.user_hotword_boost
        || before.lexicon_tags != after.lexicon_tags
}

fn privacy_data_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.privacy_enabled != after.privacy_enabled
        || before.privacy_never_learn_processes != after.privacy_never_learn_processes
        || before.privacy_never_clipboard_processes != after.privacy_never_clipboard_processes
        || before.privacy_never_candidate_processes != after.privacy_never_candidate_processes
        || before.clipboard_background_enabled != after.clipboard_background_enabled
        || before.clipboard_candidate_preview_enabled != after.clipboard_candidate_preview_enabled
        || before.clipboard_record_source_app != after.clipboard_record_source_app
        || before.clipboard_max_age_days != after.clipboard_max_age_days
        || before.clipboard_pinned_respects_max_age != after.clipboard_pinned_respects_max_age
}

fn tool_data_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.screenshot_auto_save != after.screenshot_auto_save
        || before.screenshot_copy_after_capture != after.screenshot_copy_after_capture
        || before.screenshot_ocr_after_capture != after.screenshot_ocr_after_capture
        || before.screenshot_translate_after_capture != after.screenshot_translate_after_capture
        || before.screenshot_save_dir != after.screenshot_save_dir
        || before.screenshot_silent_copy_enabled != after.screenshot_silent_copy_enabled
        || before.screenshot_silent_copy_dir != after.screenshot_silent_copy_dir
        || before.screenshot_name_pattern != after.screenshot_name_pattern
        || before.screenshot_date_subdirs != after.screenshot_date_subdirs
        || before.screenshot_conflict_strategy != after.screenshot_conflict_strategy
        || before.screenshot_format != after.screenshot_format
        || before.screenshot_mode != after.screenshot_mode
        || before.screenshot_confirm_on_release != after.screenshot_confirm_on_release
        || before.screenshot_show_instructions != after.screenshot_show_instructions
        || before.clipboard_max_history_items != after.clipboard_max_history_items
        || before.clipboard_max_pinned_items != after.clipboard_max_pinned_items
        || before.clipboard_max_text_utf16_units != after.clipboard_max_text_utf16_units
        || before.ocr_result_action != after.ocr_result_action
        || before.ocr_keep_alive != after.ocr_keep_alive
        || before.ocr_translate_keep_window != after.ocr_translate_keep_window
        || before.ocr_screenshot_auto_save != after.ocr_screenshot_auto_save
        || before.ocr_screenshot_save_dir != after.ocr_screenshot_save_dir
        || before.ocr_screenshot_name_pattern != after.ocr_screenshot_name_pattern
        || before.wintranslator_path != after.wintranslator_path
        || before.translate_result_action != after.translate_result_action
}

fn notification_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.show_notifications != after.show_notifications
        || before.show_notifications_time != after.show_notifications_time
}

fn advanced_engine_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.w_single_lm != after.w_single_lm
        || before.w_phrase_path != after.w_phrase_path
        || before.lm_single_scale != after.lm_single_scale
        || before.prefix_cache_capacity != after.prefix_cache_capacity
        || before.final_lookup_cache_capacity != after.final_lookup_cache_capacity
        || before.short_lookup_cache_capacity != after.short_lookup_cache_capacity
        || before.long_lookup_soft_budget_ms != after.long_lookup_soft_budget_ms
        || before.long_lookup_min_first_batch_candidates
            != after.long_lookup_min_first_batch_candidates
}

fn global_hotkey_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.screenshot_hotkey != after.screenshot_hotkey
        || before.clipboard_hotkey != after.clipboard_hotkey
        || before.settings_hotkey != after.settings_hotkey
        || before.handwrite_hotkey != after.handwrite_hotkey
        || before.ocr_hotkey != after.ocr_hotkey
        || before.ocr_translate_hotkey != after.ocr_translate_hotkey
        || before.translate_hotkey != after.translate_hotkey
}

fn compatibility_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.fullscreen_detection != after.fullscreen_detection
        || before.fullscreen_policy != after.fullscreen_policy
        || before.commit_transport != after.commit_transport
        || before.builtin_game_list != after.builtin_game_list
        || before.auto_suggest_app_options != after.auto_suggest_app_options
        || before.game_processes != after.game_processes
        || before.app_rules != after.app_rules
        || before.compat_rules != after.compat_rules
}

fn tool_window_runtime_settings_changed(before: &SettingsModel, after: &SettingsModel) -> bool {
    before.ocr_result_action != after.ocr_result_action
        || before.ocr_keep_alive != after.ocr_keep_alive
        || before.ocr_translate_keep_window != after.ocr_translate_keep_window
        || before.wintranslator_path != after.wintranslator_path
        || before.translate_result_action != after.translate_result_action
}

fn hotkey_conflicts(model: &SettingsModel) -> Vec<String> {
    let mut map: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (name, hotkey) in [
        ("截图", model.screenshot_hotkey.as_str()),
        ("剪贴板", model.clipboard_hotkey.as_str()),
        ("设置页", model.settings_hotkey.as_str()),
        ("手写查字", model.handwrite_hotkey.as_str()),
        ("OCR", model.ocr_hotkey.as_str()),
        ("截图翻译", model.ocr_translate_hotkey.as_str()),
        ("中英翻译", model.translate_hotkey.as_str()),
        ("简繁切换", model.traditional_hotkey.as_str()),
        ("游戏兼容", model.game_mode_hotkey.as_str()),
        ("临时英文", model.temporary_ascii_hotkey.as_str()),
    ] {
        if let Some(norm) = normalize_hotkey(hotkey) {
            map.entry(norm).or_default().push(name);
        }
    }
    if model.cn_en_hotkey == "both" || model.cn_en_hotkey == "ctrl_space" {
        map.entry("ctrl+space".to_string())
            .or_default()
            .push("中英切换");
    }
    if model.cn_en_hotkey == "both" || model.cn_en_hotkey == "shift" {
        map.entry("ctrl+shift".to_string())
            .or_default()
            .push("中英切换");
    }
    if model.full_shape_hotkey {
        map.entry("shift+space".to_string())
            .or_default()
            .push("全半角");
    }
    if model.punct_hotkey {
        map.entry("ctrl+.".to_string()).or_default().push("标点");
    }
    if model.fuzzy_hotkey {
        map.entry("ctrl+shift+f".to_string())
            .or_default()
            .push("模糊音");
    }
    if model.double_pinyin_hotkey {
        map.entry("ctrl+shift+d".to_string())
            .or_default()
            .push("双拼");
    }
    map.into_iter()
        .filter_map(|(key, names)| {
            if names.len() > 1 {
                Some(format!("{} ({})", key, names.join("/")))
            } else {
                None
            }
        })
        .collect()
}

fn normalize_hotkey(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    if lower.is_empty() || matches!(lower.as_str(), "off" | "none" | "disabled" | "关闭") {
        return None;
    }
    let mut parts: Vec<String> = lower
        .replace(['_', '-'], "+")
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| match part {
            "control" => "ctrl".to_string(),
            "," | "comma" => "comma".to_string(),
            "." | "period" | "dot" => "period".to_string(),
            " " | "space" => "space".to_string(),
            "return" => "enter".to_string(),
            "escape" => "esc".to_string(),
            "/" | "slash" => "slash".to_string(),
            ";" | "semicolon" => "semicolon".to_string(),
            "'" | "quote" | "apostrophe" => "quote".to_string(),
            "=" | "equal" | "equals" => "equal".to_string(),
            "-" | "minus" => "minus".to_string(),
            other => other.to_string(),
        })
        .collect();
    parts.sort();
    Some(parts.join("+"))
}

fn theme_label(value: &str) -> &'static str {
    match value {
        "light" => "浅色",
        "dark" => "深色",
        "high_contrast" => "高对比度",
        _ => "自动",
    }
}

fn material_label(value: &str) -> &'static str {
    match value {
        "solid" => "实心",
        "gradient" => "渐变",
        "mist" => "雾面",
        _ => "自动",
    }
}

fn density_label(value: &str) -> &'static str {
    match value {
        "compact" => "紧凑",
        "standard" => "标准",
        _ => "舒适",
    }
}

fn vertical_layout_label(value: &str) -> &'static str {
    match value {
        "compact" => "舒适",
        "card" => "卡片",
        _ => "经典",
    }
}

fn horizontal_layout_label(value: &str) -> &'static str {
    match value {
        "compact" => "单行紧凑",
        "card" => "分块卡片",
        _ => "单行舒适",
    }
}

fn screenshot_format_label(value: &str) -> &'static str {
    match value {
        "jpg" | "jpeg" => "JPG",
        _ => "PNG",
    }
}

fn ocr_result_action_label(value: &str) -> &'static str {
    match value {
        "copy" => "识别后自动复制",
        "paste" => "识别后自动粘贴",
        _ => "仅显示结果",
    }
}

fn translate_result_action_label(value: &str) -> &'static str {
    match value {
        "copy" => "翻译后自动复制",
        "paste" => "翻译后自动粘贴",
        _ => "仅显示结果",
    }
}

fn cn_en_hotkey_label(value: &str) -> &'static str {
    match value {
        "shift" => "Ctrl+Shift",
        "ctrl_space" => "Ctrl+Space",
        "none" => "关闭",
        _ => "Ctrl+Shift / Ctrl+Space",
    }
}

fn fullscreen_policy_label(value: &str) -> &'static str {
    match value {
        "show_ui" | "overlay" => "显示候选栏",
        "hide_ui" => "隐藏界面",
        "off" => "关闭",
        _ => "英文模式",
    }
}

fn commit_transport_label(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" | "game_auto" | "game-auto" | "compat_auto" | "compat-auto" => "自动",
        "global" | "inherit" | "default" => "使用全局方式",
        "clipboard_paste" | "clipboard" | "paste" => "剪贴板粘贴",
        "unicode_sendinput" | "unicode" | "sendinput" => "Unicode SendInput",
        _ => "标准 TSF",
    }
}

fn overlay_anchor_label(value: &str) -> &'static str {
    match normalize_overlay_anchor_value(value).as_str() {
        "caret" => "跟随输入光标",
        "top_left" => "屏幕左上",
        "top_center" => "屏幕上方居中",
        "top_right" => "屏幕右上",
        "bottom_left" => "屏幕左下",
        "bottom_center" => "屏幕下方居中",
        "bottom_right" => "屏幕右下",
        _ => "自动（推荐）",
    }
}

fn overlay_backend_label(value: &str) -> &'static str {
    match normalize_overlay_backend_value(value).as_str() {
        "in_process" => "进程内候选窗",
        "external" => "始终使用独立 Overlay",
        _ => "自动（全屏时独立）",
    }
}

fn overlay_monitor_label(value: &str) -> String {
    match normalize_overlay_monitor_value(value).as_str() {
        "auto" => "游戏所在显示器（推荐）".to_string(),
        "primary" => "主显示器".to_string(),
        index => index
            .parse::<usize>()
            .map(|index| format!("显示器 {}", index + 1))
            .unwrap_or_else(|_| value.trim().to_string()),
    }
}

fn merge_discovered_lexicon_tags_compat(model: &mut SettingsModel) {
    let previous = model.lexicon_tags.clone();
    merge_discovered_lexicon_tags(model);
    for (tag, enabled) in &mut model.lexicon_tags {
        if previous.contains_key(tag) {
            continue;
        }
        let legacy_values = pinyin_ime::lexicon_prefs::legacy_optional_lexicon_tags(tag)
            .iter()
            .filter_map(|legacy| previous.get(*legacy).copied())
            .collect::<Vec<_>>();
        if !legacy_values.is_empty() {
            *enabled = legacy_values.into_iter().all(|value| value);
        }
    }
}

fn lexicon_tag_label(tag: &str) -> &str {
    match tag {
        "technology" => "科技、互联网与编程",
        "daily_communication" => "聊天与办公沟通",
        "geography_admin" => "行政区划、国家与城市",
        "hangzhou_local" => "杭州本地生活与地理",
        "people_names" => "姓氏与人物姓名",
        "animals" => "常见动物",
        "medicine" => "常见药物",
        "daily_large" => "日常扩展词库",
        "county_admin" => "区县行政区划",
        "county_admin_short_names" => "区县简称",
        "hangzhou_metro" => "杭州地铁",
        "hangzhou_admin" => "杭州行政区划与街道乡镇",
        "hangzhou_transport" => "杭州交通、道路与桥隧",
        "hangzhou_public_services" => "杭州医院、学校与公共设施",
        "hangzhou_landmarks_life" => "杭州景区、历史与自然地理",
        "hangzhou_business" => "杭州商圈、园区与企业",
        "hangzhou_food_culture" => "杭帮菜、小吃与老字号",
        "hangzhou_local_culture" => "杭州文化、活动与本地表达",
        "hangzhou_places_streets" => "杭州地名街道",
        "hangzhou_scenic" => "杭州景点",
        "world_major_cities" => "世界主要城市",
        "chengyu_suyu" => "成语俗语",
        "ext" => "扩展基础词库",
        _ => tag,
    }
}

fn notification_label(value: &str) -> &'static str {
    match value {
        "false" | "0" | "off" => "关闭",
        "true" | "1" | "on" => "开启",
        _ => "自定义种类",
    }
}

struct SettingsWindowPlacement {
    inner_size: [f32; 2],
    min_inner_size: [f32; 2],
    position: Option<egui::Pos2>,
}

fn recommended_settings_window_placement() -> SettingsWindowPlacement {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{
            GetDC, GetDeviceCaps, GetMonitorInfoW, MonitorFromWindow, ReleaseDC, LOGPIXELSX,
            MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            AdjustWindowRectEx, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
        };

        fn windows_system_dpi_scale() -> f32 {
            let dc = unsafe { GetDC(0) };
            if dc == 0 {
                return 1.0;
            }
            let dpi = unsafe { GetDeviceCaps(dc, LOGPIXELSX as i32) };
            unsafe {
                ReleaseDC(0, dc);
            }
            if dpi > 0 {
                (dpi as f32 / 96.0).max(0.25)
            } else {
                1.0
            }
        }

        fn windows_primary_work_area() -> Option<(f32, f32, f32, f32)> {
            let monitor = unsafe { MonitorFromWindow(0, MONITOR_DEFAULTTOPRIMARY) };
            if monitor == 0 {
                return None;
            }
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                rcWork: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                dwFlags: 0,
            };
            if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
                return None;
            }
            let width = (info.rcWork.right - info.rcWork.left) as f32;
            let height = (info.rcWork.bottom - info.rcWork.top) as f32;
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            Some((
                info.rcWork.left as f32,
                info.rcWork.top as f32,
                width,
                height,
            ))
        }

        fn window_outer_size_for_inner(inner_size: [f32; 2], dpi_scale: f32) -> [f32; 2] {
            let inner_width = (inner_size[0] * dpi_scale).round().max(1.0);
            let inner_height = (inner_size[1] * dpi_scale).round().max(1.0);
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: inner_width as i32,
                bottom: inner_height as i32,
            };
            if unsafe { AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, WS_EX_APPWINDOW) }
                == 0
            {
                return [inner_width, inner_height];
            }
            [
                (rect.right - rect.left).max(1) as f32,
                (rect.bottom - rect.top).max(1) as f32,
            ]
        }

        if let Some((work_left, work_top, work_width_px, work_height_px)) =
            windows_primary_work_area()
        {
            let dpi_scale = windows_system_dpi_scale();
            let work_width = work_width_px / dpi_scale;
            let work_height = work_height_px / dpi_scale;
            let max_width = SETTINGS_MAX_WINDOW_SIZE[0].min((work_width - 32.0).max(1.0));
            let min_width = SETTINGS_MIN_WINDOW_SIZE[0].min(max_width);
            let width = (work_width * 0.72).clamp(min_width, max_width);
            let outer_for_width = window_outer_size_for_inner([width, work_height], dpi_scale);
            let chrome_height = (outer_for_width[1] - work_height_px).max(0.0);
            let height = ((work_height_px - chrome_height).max(1.0) / dpi_scale).min(work_height);
            let min_height = SETTINGS_MIN_WINDOW_SIZE[1].min(height);
            let inner_size = [width, height];
            let outer_size = window_outer_size_for_inner(inner_size, dpi_scale);
            let x = work_left + ((work_width_px - outer_size[0]) * 0.5).max(0.0);
            let y = work_top;
            return SettingsWindowPlacement {
                inner_size,
                min_inner_size: [min_width, min_height],
                position: Some(egui::pos2(x / dpi_scale, y / dpi_scale)),
            };
        }
    }

    SettingsWindowPlacement {
        inner_size: SETTINGS_WINDOW_SIZE,
        min_inner_size: SETTINGS_MIN_WINDOW_SIZE,
        position: None,
    }
}

fn settings_viewport() -> egui::ViewportBuilder {
    let placement = recommended_settings_window_placement();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(placement.inner_size)
        .with_min_inner_size(placement.min_inner_size)
        .with_title(WINDOW_TITLE);
    if let Some(position) = placement.position {
        viewport = viewport.with_position(position);
    }
    viewport
}

#[cfg(windows)]
struct SettingsInstanceGuard {
    _handle: Option<pinyin_ime::win_handle::OwnedWinHandle>,
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn activate_existing_settings_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    let title = wide_null(WINDOW_TITLE);
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd != 0 {
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(windows)]
fn acquire_settings_instance() -> Option<SettingsInstanceGuard> {
    use pinyin_ime::win_handle::OwnedWinHandle;
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name = wide_null("Local\\KaixinImeSettings.SingleInstance");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle == 0 {
        return Some(SettingsInstanceGuard { _handle: None });
    }
    // SAFETY: CreateMutexW returned a new mutex handle owned here.
    let handle = unsafe { OwnedWinHandle::try_from_raw(handle) }.expect("validated settings mutex");
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        drop(handle);
        activate_existing_settings_window();
        return None;
    }
    Some(SettingsInstanceGuard {
        _handle: Some(handle),
    })
}

#[cfg(not(windows))]
struct SettingsInstanceGuard;

#[cfg(not(windows))]
fn acquire_settings_instance() -> Option<SettingsInstanceGuard> {
    Some(SettingsInstanceGuard)
}

fn main() -> eframe::Result<()> {
    pinyin_ime::windows_security::apply_process_hardening();
    let Some(_instance_guard) = acquire_settings_instance() else {
        return Ok(());
    };
    let options = eframe::NativeOptions {
        viewport: settings_viewport(),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(SettingsApp::new(cc)))),
    )
}
