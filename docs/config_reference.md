# Configuration Reference

This file documents the main `kaixin.ini` keys that are shared across the
settings app, TSF front end, tray, and engine. Keep this table in sync when a
new setting is added or a default changes.

| Section | Key | Default | Range / Values | Readers |
| --- | --- | --- | --- | --- |
| `general` | `config_version` | `5` | integer migration marker | installer, settings |
| `diagnostics` | `log_level` | `basic` | `off`, `error`, `basic`, `perf`, `verbose` | Rust logs, TSF logs |
| `style` | `candidate_page_size` | `9` | `3..=9` in settings; vertical quick panels may force `10` | settings, TSF, engine ranking |
| `style` | `candidate_horizontal` | `1` | boolean | settings, TSF candidate window |
| `style` | `candidate_horizontal_count` | `5` | `3..=9` | settings, TSF candidate window |
| `style` | `candidate_font_size` | `16` | `14..=28`; skins can recommend it but saved user settings win | settings, TSF candidate window |
| `style` | `candidate_font_file` | `Microsoft YaHei` | font face or local font path; empty resolves to `Microsoft YaHei`; no HarmonyOS Sans font is bundled in release packages | settings, TSF candidate window |
| `style` | `candidate_font_weight` | `500` | `300..=700` | settings, TSF candidate window |
| `style` | `candidate_selected_font_weight` | `600` | `400..=800` | settings, TSF candidate window |
| `style` | `candidate_label_font_weight` | `600` | `400..=800` | settings, TSF candidate window |
| `style` | `candidate_chip_font_weight` | `500` | `350..=700` | settings, TSF candidate window |
| `style` | `candidate_opacity` | `100` | `90..=100` | settings, TSF candidate window |
| `style` | `theme` | `auto` | `auto`, `light`, `dark`, `high_contrast` | settings, TSF |
| `style` | `candidate_material` | `auto` | `auto`, `solid`, `gradient`, `mist` | settings, TSF |
| `style` | `candidate_layout_variant` | legacy fallback | `classic`, `compact`, `card` | TSF, settings migration |
| `style` | `candidate_vertical_layout_variant` | `compact` | `classic`, `compact`, `card` | settings, TSF |
| `style` | `candidate_horizontal_layout_variant` | `classic` | `classic`, `compact`, `card` | settings, TSF |
| `style` | `highlight_typo_candidates` | `1` | boolean; prefixes correction candidates with `~` | settings, TSF |
| `style` | `show_candidate_reading` | `0` | boolean; shows the current reading/pinyin in candidate comments | settings, TSF |
| `style` | `show_candidate_score` | `0` | boolean; shows ranking score in candidate comments | settings, TSF |
| `style` | `show_candidate_source` | `0` | boolean; prefixes visible source tags for pinned/user/ext/correction candidates | settings, TSF |
| `compatibility` | `fullscreen_detection` | `1` | boolean | settings, TSF |
| `compatibility` | `fullscreen_policy` | `show_ui` | `show_ui`, `ascii`, `hide_ui`, `off` | settings, TSF |
| `compatibility` | `commit_transport` | `tsf` | `auto`, `tsf`, `clipboard_paste`, `unicode_sendinput` | settings, TSF |
| `compatibility` | `builtin_game_list` | `1` | boolean | settings, TSF |
| `compatibility` | `auto_suggest_app_options` | `1` | boolean | settings, TSF |
| `compatibility` | `game_processes` | empty | comma/newline separated wildcard process names; matching apps use `fullscreen_policy` | settings, TSF |
| `app:<process>` | `focus_policy` | `normal` | `normal`, `strict`, `window` | settings, TSF |
| `app:<process>` | `game_profile` | empty | `compact` enables the game candidate profile | settings, TSF |
| `app:<process>` | `overlay_anchor` | `auto` | `auto`, `caret`, `top_left`, `top_center`, `top_right`, `bottom_left`, `bottom_center`, `bottom_right` | settings, TSF |
| `app:<process>` | `overlay_offset_x` / `overlay_offset_y` | `0` | `-4000..=4000` logical pixels | settings, TSF |
| `app:<process>` | `overlay_scale` | `100` | `50..=200` percent; decimal scale such as `1.25` is accepted as `125` by settings | settings, TSF |
| `app:<process>` | `overlay_monitor` | `auto` | `auto`, `primary`, or zero-based monitor index | settings, TSF |
| `app:<process>` | `overlay_backend` | `auto` | `auto`, `in_process`, `external`; auto uses the independent helper only for fullscreen or UI-less hosts, `external` always requests it | settings, TSF |
| `clipboard` | `background_enabled` | `1` | boolean | settings, engine, clipboard manager |
| `clipboard` | `max_history_items` | `60` | `0..=300` | settings, engine, clipboard manager |
| `clipboard` | `max_pinned_items` | `24` | `0..=100` | settings, engine, clipboard manager |
| `clipboard` | `max_text_utf16_units` | `20000` | `20..=20000` | settings, engine, clipboard manager |
| `clipboard` | `max_age_days` | `0` | `0..=3650`; used by manual age cleanup, `0` disables age cleanup | settings, clipboard manager |
| `clipboard` | `candidate_preview_enabled` | `0` | boolean; shows clipboard text snippets in candidate metadata | settings, engine |
| `clipboard` | `record_source_app` | `0` | boolean; persists source process path in clipboard history | settings, engine, clipboard manager |
| `clipboard` | `pinned_respects_max_age` | `1` | boolean; manual age cleanup also removes pinned clipboard entries | settings, clipboard manager |
| `clipboard` | `hotkey` | `off` | Windows hotkey or `off` | settings, tray |
| `privacy` | `never_learn_processes` | empty | comma/newline separated wildcard process names | settings, TSF |
| `privacy` | `never_clipboard_processes` | empty | comma/newline separated wildcard process names | settings, TSF |
| `privacy` | `never_candidate_processes` | empty | comma/newline separated wildcard process names | settings, TSF |
| `screenshot` | `hotkey` | `off` | Windows hotkey or `off` | settings, tray |
| `screenshot` | `auto_save` | `1` | boolean; saves the final image to `save_dir` using `name_pattern` | settings, tray |
| `screenshot` | `copy_after_capture` | `1` | boolean; copies the final image to the system clipboard | settings, tray |
| `screenshot` | `open_editor_after_capture` | `1` | boolean; ShareX only, opens the annotation editor after selection; disable for direct save/copy | settings, tray, ShareX |
| `screenshot` | `save_dir` | empty | folder path; empty uses `%USERPROFILE%\Pictures\Kaixin Screenshots` | settings, tray |
| `screenshot` | `silent_copy_enabled` | `0` | boolean; after primary auto-save succeeds, also copy the file to `silent_copy_dir` | settings, tray |
| `screenshot` | `silent_copy_dir` | empty | folder path for silent screenshot copies; empty disables the copy target | settings, tray |
| `screenshot` | `name_pattern` | `{timestamp}` | filename stem pattern; `{timestamp}` is local time with milliseconds; also supports `{date}`, `{time}`, `{datetime}`, `{seq}`, `{app}`, `{window}`, `{width}`, `{height}` | settings, tray |
| `screenshot` | `date_subdirs` | `0` | boolean; saves into `YYYY/MM/DD` subdirectories below `save_dir` | settings, tray |
| `screenshot` | `conflict_strategy` | `increment` | `increment` appends `_002` etc.; `overwrite` replaces an existing same-name file | settings, tray |
| `screenshot` | `format` | `png` | `png`, `jpg` | settings, tray |
| `screenshot` | `backend` | `sharex` | `sharex` uses the bundled ShareX capture/edit flow; `wgc` uses the native Windows Graphics Capture path | settings, tray |
| `screenshot` | `mode` | `manual_region` | `manual_region` or `current_window`; ShareX defaults to manual region capture | settings, tray |
| `sharex` | `detect_windows` / `detect_controls` | `1` / `1` | snap region selection to windows and child controls | settings, tray, ShareX |
| `sharex` | `show_magnifier` / `show_info` / `show_crosshair` | `1` / `1` / `0` | region-selection helpers | settings, tray, ShareX |
| `sharex` | `magnifier_pixel_count` / `magnifier_pixel_size` | `15` / `160` | source pixels `1..=100`, display size `40..=1000` | settings, tray, ShareX |
| `sharex` | `magnifier_square` / `show_center_crosshair` | `0` / `0` | square magnifier and center marker toggles | settings, tray, ShareX |
| `sharex` | `use_dimming` / `dim_strength` | `1` / `20` | dim unselected screen area; strength `0..=100` | settings, tray, ShareX |
| `sharex` | `enable_animations` | `1` | region-selection animations | settings, tray, ShareX |
| `sharex` | `fixed_size_enabled` / `fixed_width` / `fixed_height` | `0` / `250` / `250` | fixed-size region selection | settings, tray, ShareX |
| `sharex` | `show_cursor` / `screenshot_delay` | `1` / `0` | include cursor and delay capture by seconds | settings, tray, ShareX |
| `sharex` | `capture_client_area` / `capture_shadow` | `0` / `1` | current-window capture options | settings, tray, ShareX |
| `sharex` | `hide_taskbar` / `hide_desktop_icons` | `0` / `0` | temporarily hide desktop surfaces during capture | settings, tray, ShareX |
| `sharex` | `jpeg_quality` | `90` | JPEG encoder quality `1..=100` | settings, tray, ShareX |
| `sharex` | `open_folder_after_capture` / `pin_to_screen` / `show_notification` | `0` / `0` / `0` | local post-capture actions | settings, tray, ShareX |
| `sharex` | `editor_annotation_color` / `editor_thickness` | `#F23C3C` / `4` | default shape annotation color and line width | settings, tray, ShareX |
| `sharex` | `editor_text_color` / `editor_text_border_color` | `#FFFFFF` / `#F23C3C` | independent text foreground and outline colors | settings, tray, ShareX |
| `sharex` | `editor_font_family` / `editor_font_size` | `Segoe UI` / `48` | default editor text style | settings, tray, ShareX |
| `sharex` | `editor_arrow_style` | `classic` | `classic`, `modern`, `double`, `basic`, `line` | settings, tray, ShareX |
| `sharex` | `editor_blur_strength` / `editor_pixelate_strength` | `30` / `20` | editor effect strengths | settings, tray, ShareX |
| `sharex` | `editor_step_type` | `numeric` | numeric, letter, or Roman step markers | settings, tray, ShareX |
| `sharex` | `editor_auto_close` / `editor_remember_last_tool` | `0` / `1` | editor completion and tool-memory behavior | settings, tray, ShareX |
| `sharex` | `editor_default_tool` | `rectangle` | default annotation tool when tool memory is disabled | settings, tray, ShareX |
| `sharex` | `editor_toolbar_tools` | all tools | comma-separated visible editor tool IDs | settings, tray, ShareX |
| `tools` | `settings_hotkey` | `off` | Windows hotkey or `off` | settings, tray |
| `tools` | `handwrite_hotkey` | `off` | Windows hotkey or `off` | settings, tray |
| `tools` | `ocr_hotkey` | `off` | Windows hotkey or `off` | settings, tray |
| `tools` | `ocr_translate_hotkey` | `off` | Windows hotkey or `off` | settings, tray |
| `tools` | `ocr_result_action` | `show` | `show`, `copy`, `paste` | settings, OCR |
| `tools` | `ocr_translate_keep_window` | `1` | boolean | settings, OCR |
| `tools` | `translate_hotkey` | `off` | Windows hotkey or `off` | settings, tray |
| `tools` | `translate_result_action` | `show` | `show`, `copy`, `paste` | settings, translate |
| `lexicon` | `lexicon_<tag>` | `1` | boolean; controls optional text dictionaries by tag | settings, engine |
| `input` | `traditional_output` | `0` | boolean | settings, TSF, engine ranking |
| `input` | `traditional_hotkey` | `off` | Ctrl/Shift/Alt + key or `off` | settings, TSF |
| `input` | `game_mode_hotkey` | `off` | Ctrl/Shift/Alt + key or `off` | settings, TSF |
| `input` | `temporary_ascii_hotkey` | `off` | Ctrl/Shift/Alt + key or `off` | settings, TSF |
| `input` | `shift_tap_hotkey` | `1` | boolean | settings, TSF |
| `input` | `candidate_number_select` | `1` | boolean | settings, TSF |
| `input` | `date_auto_format` | `1` | boolean | settings, TSF, engine ranking |
| `input` | `symbol_toolbox` | `1` | boolean | settings, TSF, engine ranking |
| `input` | `emoji_input` | `1` | boolean | settings, TSF, engine ranking |
| `input` | `auto_pair_punct` | `1` | boolean | settings, TSF |
| `input` | `symbol_fullwidth` | `1` | boolean | settings, TSF |
| `input` | `shift_symbol_temporary_ascii` | `0` | boolean | settings, TSF |
| `input` | `page_minus_equal` | `1` | boolean | settings, TSF |
| `input` | `page_comma_period` | `1` | boolean | settings, TSF |
| `engine` | `retry_on_failure` | `1` | boolean | TSF |
| `engine` | `learning_sensitivity` | `standard` | `conservative`, `standard`, `aggressive` | settings, TSF, engine learning |
| `engine` | `user_hotword_boost` | `standard` | `conservative`, `standard`, `strong`, `aggressive` | settings, engine ranking |
| `engine` | `prefix_cache_capacity` | `384` | `8..=512` | engine lookup |
| `engine` | `final_lookup_cache_capacity` | `128` | `8..=512` | engine lookup |
| `engine` | `short_lookup_cache_capacity` | `192` | `8..=512` | engine lookup |
| `engine` | `long_lookup_soft_budget_ms` | `4` | `1..=50`; long input first-page deadline | engine lookup |
| `engine` | `long_lookup_min_first_batch_candidates` | `6` | `1..=128`; minimum visible candidates before deadline applies | engine lookup |

Shared path and process naming rules:

| Rule | Value |
| --- | --- |
| App data directory | `%LOCALAPPDATA%\kaixin` |
| Config path | `%LOCALAPPDATA%\kaixin\kaixin.ini` |
| Log directory | `%LOCALAPPDATA%\kaixin\logs` |
| IPC capability token | `%LOCALAPPDATA%\kaixin\engine_capability.dat` |
| User dictionary database | `%LOCALAPPDATA%\kaixin\user_dict.sqlite` (local SQLite protected with Windows DPAPI) |
| Clipboard history database | `%LOCALAPPDATA%\kaixin\clipboard_store.sqlite` (local SQLite protected with Windows DPAPI) |
| OCR history database | `%LOCALAPPDATA%\kaixin\ocr_history.sqlite` (local SQLite protected with Windows DPAPI) |
| Screenshot library index | `%LOCALAPPDATA%\kaixin\screenshot_library.sqlite` |
| Runtime event database | `%LOCALAPPDATA%\kaixin\runtime_events.sqlite` (structured diagnostics mirror; text logs are still written under `logs`) |
| Engine pipe base | `\\.\pipe\KaixinInput_Engine_V5` |
| Engine mutex base | `Local\KaixinInput_Engine_Mutex_V5` |
| Engine suffix | FNV-1a 64-bit over lower-case UTF-16 install root, after removing `\\?\` / `\\?\UNC\` prefixes |

Fullscreen display notes:

| Rule | Value |
| --- | --- |
| Fullscreen default | `[compatibility] fullscreen_policy=show_ui` uses the compact, click-through game candidate overlay; `hide_ui` remains available as an explicit compatibility fallback. |
| Fullscreen candidate overlay | Set `[compatibility] fullscreen_policy=show_ui` to keep candidates visible in fullscreen without forcing ASCII. |
| Per-game candidate overlay | Set `[app:<process>] policy=show_ui`, or equivalently `ascii_mode=0`, `hide_ui=0`, and `candidate_topmost=1`. |
| Recommended game profile | The settings app writes `policy=show_ui`, `game_profile=compact`, `commit_transport=tsf`, and `overlay_backend=auto`; the test wizard can then switch to Unicode SendInput or clipboard paste as real fallback steps. |
| Per-game overlay placement | Use `overlay_anchor`, logical-pixel offsets, percentage scale, and `overlay_monitor`; `auto` follows the game window/display. |
| Overlay backend selection | `auto` keeps low-latency in-process rendering for ordinary windowed games and switches to the independent helper for fullscreen/UI-less hosts; use `external` to force the helper during troubleshooting. |
| Per-game commit fallback | Set `[app:<process>] commit_transport=clipboard_paste` or `unicode_sendinput` only for games that do not accept TSF commits. |
| Best target | Borderless/windowed fullscreen games; exclusive fullscreen may prevent ordinary topmost windows from appearing above the game. |

Font and clipboard notes:

| Rule | Value |
| --- | --- |
| Default UI font | Microsoft YaHei is preferred by the settings app, clipboard manager, and candidate window. |
| Font precedence | Explicit user `candidate_font_file` wins over the Microsoft YaHei default; skin `font_file` is only a recommendation when no user font was saved. |
| Clipboard background capture | Disabled by default. Set `[clipboard] background_enabled=1` to opt in to a listener/poller that writes text history to the local DPAPI-protected SQLite database on Windows. |
| Global privacy mode | `[privacy] enabled=1` forces ASCII input, suppresses candidates and learning, and prevents clipboard capture or history disclosure. |
| Clipboard on-demand capture | Opening the clipboard manager, pressing refresh, or using `vvu` reads the current system text clipboard once even when background capture is off. |
