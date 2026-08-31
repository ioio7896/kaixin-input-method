#pragma once

namespace SrfConfigSchema {

namespace section {
inline constexpr wchar_t kStyle[] = L"style";
inline constexpr wchar_t kCompatibility[] = L"compatibility";
inline constexpr wchar_t kPrivacy[] = L"privacy";
}  // namespace section

namespace key {
inline constexpr wchar_t kTheme[] = L"theme";
inline constexpr wchar_t kCandidateMaterial[] = L"candidate_material";
inline constexpr wchar_t kCandidateDensity[] = L"candidate_density";
inline constexpr wchar_t kCandidateLayoutVariant[] = L"candidate_layout_variant";
inline constexpr wchar_t kCandidateVerticalLayoutVariant[] = L"candidate_vertical_layout_variant";
inline constexpr wchar_t kCandidateHorizontalLayoutVariant[] = L"candidate_horizontal_layout_variant";
inline constexpr wchar_t kCandidateReduceMotion[] = L"candidate_reduce_motion";
inline constexpr wchar_t kHighlightTypoCandidates[] = L"highlight_typo_candidates";
inline constexpr wchar_t kShowCandidateSource[] = L"show_candidate_source";

inline constexpr wchar_t kFullscreenDetection[] = L"fullscreen_detection";
inline constexpr wchar_t kFullscreenPolicy[] = L"fullscreen_policy";
inline constexpr wchar_t kCommitTransport[] = L"commit_transport";
inline constexpr wchar_t kGameProfile[] = L"game_profile";
inline constexpr wchar_t kOverlayAnchor[] = L"overlay_anchor";
inline constexpr wchar_t kOverlayOffsetX[] = L"overlay_offset_x";
inline constexpr wchar_t kOverlayOffsetY[] = L"overlay_offset_y";
inline constexpr wchar_t kOverlayScale[] = L"overlay_scale";
inline constexpr wchar_t kOverlayMonitor[] = L"overlay_monitor";
inline constexpr wchar_t kOverlayBackend[] = L"overlay_backend";
inline constexpr wchar_t kBuiltinGameList[] = L"builtin_game_list";
inline constexpr wchar_t kAutoSuggestAppOptions[] = L"auto_suggest_app_options";
inline constexpr wchar_t kGameProcesses[] = L"game_processes";
inline constexpr wchar_t kHotkeyScope[] = L"hotkey_scope";

inline constexpr wchar_t kNeverLearnProcesses[] = L"never_learn_processes";
inline constexpr wchar_t kNeverClipboardProcesses[] = L"never_clipboard_processes";
inline constexpr wchar_t kNeverCandidateProcesses[] = L"never_candidate_processes";
inline constexpr wchar_t kPrivacyEnabled[] = L"enabled";
}  // namespace key

namespace defaults {
inline constexpr wchar_t kTheme[] = L"auto";
inline constexpr wchar_t kCandidateMaterial[] = L"auto";
inline constexpr wchar_t kCandidateDensity[] = L"standard";
inline constexpr wchar_t kCandidateLayoutVariant[] = L"compact";
inline constexpr wchar_t kCandidateHorizontalLayoutVariant[] = L"classic";
inline constexpr wchar_t kFullscreenPolicy[] = L"show_ui";
inline constexpr wchar_t kCommitTransport[] = L"tsf";
}  // namespace defaults

}  // namespace SrfConfigSchema
