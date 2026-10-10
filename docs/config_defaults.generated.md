# Generated configuration defaults

Edit `shared/config_schema.json`, then run `python scripts/generate_shared_contracts.py`. Existing user values are preserved. Unknown keys remain available for extensions.

| Section | Key | Default | Type | Options |
|---|---|---|---|---|
| diagnostics | log_level | error | string |  |
| style | candidate_horizontal | 1 | bool |  |
| style | candidate_density | standard | string | compact, standard, comfortable |
| style | candidate_vertical_layout_variant | compact | string |  |
| style | candidate_horizontal_layout_variant | classic | string |  |
| compatibility | fullscreen_detection | 1 | bool |  |
| compatibility | fullscreen_policy | show_ui | string | show_ui, ascii, hide_ui, off |
| compatibility | game_input_mode | manual | string | manual, passthrough, chinese, auto_text |
| compatibility | commit_transport | tsf | string | auto, tsf, clipboard_paste, unicode_sendinput |
| compatibility | builtin_game_list | 1 | bool |  |
| compatibility | auto_suggest_app_options | 1 | bool |  |
| privacy | never_learn_processes |  | string |  |
| privacy | never_clipboard_processes |  | string |  |
| privacy | never_candidate_processes |  | string |  |
| clipboard | background_enabled | 0 | bool |  |
| clipboard | max_history_items | 60 | integer |  |
| clipboard | max_pinned_items | 24 | integer |  |
| clipboard | max_text_utf16_units | 20000 | integer |  |
| clipboard | max_age_days | 0 | bool |  |
| screenshot | hotkey | off | string |  |
| screenshot | auto_save | 1 | bool |  |
| screenshot | save_dir |  | string |  |
| screenshot | silent_copy_enabled | 0 | bool |  |
| screenshot | silent_copy_dir |  | string |  |
| screenshot | name_pattern | {timestamp} | string |  |
| screenshot | format | png | string |  |
| screenshot | copy_after_capture | 1 | bool |  |
| screenshot | ocr_after_capture | 0 | bool |  |
| screenshot | translate_after_capture | 0 | bool |  |
| screenshot | mode | manual_region | string |  |
| screenshot | confirm_on_release | 0 | bool |  |
| screenshot | show_instructions | 1 | bool |  |
| clipboard | hotkey | off | string |  |
| tools | settings_hotkey | off | string |  |
| tools | handwrite_hotkey | off | string |  |
| tools | ocr_hotkey | off | string |  |
| tools | translate_hotkey | off | string |  |
| input | traditional_hotkey | off | string |  |
| input | game_mode_hotkey | Ctrl+Shift+Alt+G | string |  |
| input | temporary_ascii_hotkey | off | string |  |
| input | shift_tap_hotkey | 1 | bool |  |
| input | candidate_number_select | 1 | bool |  |
| input | symbol_fullwidth | 0 | bool |  |
| input | shift_symbol_temporary_ascii | 0 | bool |  |
| input | page_minus_equal | 1 | bool |  |
| input | page_comma_period | 1 | bool |  |
| engine | retry_on_failure | 1 | bool |  |
| engine | long_lookup_soft_budget_ms | 4 | integer |  |
| general | config_version | 14 | integer |  |
| input | default_ascii | 0 | bool |  |
| general | global_ascii | 0 | bool |  |
| input | default_full_shape | 0 | bool |  |
| input | default_chinese_punct | 1 | bool |  |
| input | curly_punct | 1 | bool |  |
| input | auto_pair_punct | 1 | bool |  |
| input | number_fullwidth | 0 | bool |  |
| input | date_auto_format | 1 | bool |  |
| input | english_word_input | 0 | bool |  |
| input | traditional_output | 0 | bool |  |
| input | default_fuzzy_pinyin | 0 | bool |  |
| fuzzy | zh_z | 1 | bool |  |
| fuzzy | ch_c | 1 | bool |  |
| fuzzy | sh_s | 1 | bool |  |
| fuzzy | n_l | 1 | bool |  |
| fuzzy | f_h | 1 | bool |  |
| fuzzy | an_ang | 1 | bool |  |
| fuzzy | en_eng | 1 | bool |  |
| fuzzy | in_ing | 1 | bool |  |
| input | default_double_pinyin | 0 | bool |  |
| engine | jianpin | 1 | bool |  |
| engine | mixed_pinyin | 1 | bool |  |
| engine | mixed_pinyin_aggressive | 0 | bool |  |
| engine | v_assist | 1 | bool |  |
| input | symbol_toolbox | 1 | bool |  |
| input | emoji_input | 1 | bool |  |
| engine | u_mode | 0 | bool |  |
| engine | show_status_notifications | 1 | bool |  |
| correction | enabled | 1 | bool |  |
| style | inline_preedit | 1 | bool |  |
| style | enhanced_position | 1 | bool |  |
| style | paging_on_scroll | 1 | bool |  |
| style | candidate_page_size | 9 | integer |  |
| style | candidate_horizontal_count | 5 | integer |  |
| style | candidate_horizontal_compact | 0 | bool |  |
| style | candidate_font_size | 16 | integer |  |
| style | candidate_opacity | 100 | integer |  |
| style | candidate_reduce_motion | 0 | bool |  |
| style | candidate_font_weight | 500 | integer |  |
| style | candidate_selected_font_weight | 600 | integer |  |
| style | candidate_label_font_weight | 600 | integer |  |
| style | candidate_chip_font_weight | 500 | integer |  |
| style | candidate_topmost | 1 | bool |  |
| style | show_candidate_reading | 0 | bool |  |
| style | show_candidate_score | 0 | bool |  |
| style | show_mode_in_candidate_header | 0 | bool |  |
| style | candidate_abbreviate_length | 64 | integer |  |
| input | full_shape_hotkey | 0 | bool |  |
| input | punct_hotkey | 0 | bool |  |
| input | fuzzy_hotkey | 0 | bool |  |
| input | double_pinyin_hotkey | 0 | bool |  |
| input | candidate_left_click | 1 | bool |  |
| input | candidate_right_click | 1 | bool |  |
| input | page_pgup_pgdn | 1 | bool |  |
| screenshot | date_subdirs | 0 | bool |  |
| ocr | keep_alive | 1 | bool |  |
| tools | ocr_translate_keep_window | 1 | bool |  |
| ocr | screenshot_auto_save | 1 | bool |  |
| clipboard | candidate_preview_enabled | 0 | bool |  |
| clipboard | record_source_app | 0 | bool |  |
| clipboard | pinned_respects_max_age | 1 | bool |  |
| general | show_notifications_time | 1200 | integer |  |
| style | theme | auto | string | auto, light, dark, high_contrast |
| style | candidate_material | auto | string | auto, solid, gradient, mist |
| engine | final_lookup_cache_capacity | 128 | integer |  |
| engine | learning_sensitivity | standard | string |  |
| engine | long_lookup_min_first_batch_candidates | 6 | integer |  |
| engine | prefix_cache_capacity | 384 | integer |  |
| engine | short_lookup_cache_capacity | 192 | integer |  |
| engine | user_hotword_boost | standard | string |  |
| general | show_notifications | true | string |  |
| input | cn_en_hotkey | none | string |  |
| input | double_pinyin_schema | builtin | string |  |
| ocr | profile | balanced | string |  |
| ocr | screenshot_name_pattern | ocr-{datetime} | string |  |
| ocr | screenshot_save_dir |  | string |  |
| privacy | enabled | 0 | bool |  |
| rank | lm_single_scale | 3.350 | string |  |
| rank | w_phrase_path | 0.160 | string |  |
| rank | w_single_lm | 0.820 | string |  |
| screenshot | conflict_strategy | increment | string |  |
| style | candidate_font_file | Microsoft YaHei | string |  |
| style | candidate_skin_file |  | string |  |
| style | highlight_typo_candidates | 1 | bool |  |
| style | show_candidate_source | 0 | bool |  |
| tools | ocr_result_action | show | string |  |
| tools | ocr_translate_hotkey | off | string |  |
| tools | translate_result_action | show | string |  |
| tools | wintranslator_path |  | string |  |
| tools | translate_target_language | auto-opposite | string |  |
| compatibility | game_enter_behavior | auto | string | auto, close, stay |
| compatibility | game_auto_uia | 1 | bool |  |
| compatibility | game_status_indicator | 1 | bool |  |
