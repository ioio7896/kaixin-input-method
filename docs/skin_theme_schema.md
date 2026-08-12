# Candidate Skin Theme Schema

Skin files live at `skins/<name>/theme.json`. The settings app scans this
directory and the TSF candidate window loads the selected theme at runtime.

## Basic Fields

| Key | Type | Notes |
| --- | --- | --- |
| `name` | string | Display name for humans. |
| `material` | string | `auto`, `solid`, `gradient`, or `mist` (`mist` is a soft gradient treatment, not backdrop blur). |
| `layout` | string | `classic`, `compact`, or `card`. Used only when the user has not explicitly chosen separate vertical/horizontal layouts. |
| `font_size` | integer | 14..28. Ignored when the user has saved a candidate font size in settings. |
| `font_file` | string | Font face/path. Ignored when the user has saved a candidate font in settings. |

## Colors

Colors use `#RRGGBB`.

| Key | Purpose |
| --- | --- |
| `window_bg`, `window_bg_to` | Candidate window background and optional vertical gradient end. |
| `header_bg`, `header_bg_to` | Vertical layout header background and optional gradient end. |
| `border`, `divider` | Outer border and row divider. |
| `item_bg`, `item_border` | Normal candidate row/card. |
| `hover_bg`, `hover_border` | Mouse hover candidate row/card. |
| `selected_bg`, `selected_border` | Selected candidate row/card. |
| `pressed_bg`, `pressed_border` | Mouse pressed candidate row/card. |
| `text`, `muted_text` | Normal candidate text and metadata text. |
| `selected_text`, `selected_muted_text` | Selected candidate text and metadata text. |
| `badge_bg`, `badge_border`, `badge_text` | Candidate number badge and page badge. |
| `chip_bg`, `chip_border`, `chip_text` | Mode chip. |
| `chip_active_bg`, `chip_active_border`, `chip_active_text` | Active mode chip. |
| `selected_outline` | Accent color for selected indicator and selected number badge. |

Runtime will correct text colors when contrast is too low. `build.py` also
prints warnings for low-contrast theme pairs so theme files can match runtime
display more closely.

## Selected State

| Key | Type | Range / Values | Notes |
| --- | --- | --- | --- |
| `selected_accent_width` | integer | 0..8 | Accent bar thickness in logical pixels. |
| `selected_ring_opacity` | number | 0.0..1.0 | Optional inner selected ring blended over selected background. |
| `selected_indicator` | string | `left_bar`, `bottom_bar`, `outline`, `none` | Optional override. If omitted, vertical layout uses `left_bar` and horizontal layout uses `bottom_bar`. |

## Shape And Spacing

| Key | Type | Notes |
| --- | --- | --- |
| `corner_radius` | integer | Outer window radius. Also controls the real window region. |
| `header_corner_radius` | integer | Header radius. |
| `row_corner_radius` | integer | Candidate row/card radius. |
| `badge_corner_radius` | integer | Number badge and chip radius. |
| `outer_pad_x`, `outer_pad_y` | integer | Window padding. |
| `header_pad_x`, `header_pad_y`, `header_gap` | integer | Vertical header spacing. |
| `item_gap`, `item_pad_x`, `item_pad_y` | integer | Candidate spacing. |
| `label_width`, `label_gap`, `comment_gap` | integer | Number badge and metadata spacing. |
| `min_width`, `preferred_width`, `max_width` | integer | Vertical window width hints. |
| `min_horizontal_card_width`, `max_horizontal_card_width` | integer | Horizontal candidate width hints. |

## Effects And Weights

| Key | Type | Range / Values |
| --- | --- | --- |
| `border_opacity` | number | 0.0..1.0 |
| `divider_opacity` | number | 0.0..1.0 |
| `shadow_enabled` | boolean | `true` or `false` |
| `shadow_opacity` | number | Keep subtle for daily-use skins. |
| `shadow_size` | integer | `0..24` logical pixels. |
| `font_weight` | integer | 300..700 |
| `selected_font_weight` | integer | 400..800 |
| `label_font_weight` | integer | 400..800 |
| `chip_font_weight` | integer | 350..700 |

## Motion

Candidate motion is disabled when the user enables “Reduce motion”, Windows
client-area animations are disabled, high-contrast mode is active, or the
candidate window is running as a game overlay. Durations are logical
milliseconds and are clamped to `0..240`; `0` disables that transition.

| Key | Type | Default | Purpose |
| --- | --- | --- | --- |
| `animations_enabled` | boolean | `true` | Disable all motion for this skin. |
| `show_animation_ms` | integer | `90` | Initial fade and 2px settle. |
| `selection_animation_ms` | integer | `80` | Selection color and indicator movement. |
| `hover_animation_ms` | integer | `70` | Mouse hover color transition. |
| `press_animation_ms` | integer | `36` | Mouse press/release transition. |
| `page_animation_ms` | integer | `110` | Directional 6px page-entry transition. |
