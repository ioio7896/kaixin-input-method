# High DPI GDI Checklist

Use this checklist when validating candidate and notification windows after GDI/DPI changes.

## Scale Matrix

- 100%, 125%, 150%, 175%, 200%, 250% on a single monitor.
- 100% primary + 200% secondary, candidate near the secondary edge.
- 200% primary + 100% secondary, candidate near the secondary edge.
- Remote Desktop attach/detach with a different client scale.
- Change Windows scale while a host app is already running, then trigger candidates again.

## Candidate Window

- Vertical and horizontal layouts keep text inside rows/cards.
- Selected, hover, pressed, and pin menu states do not reuse stale rectangles after crossing monitors.
- Borders and dividers remain visible at 150% and above without looking heavy at 100%.
- Rounded corners do not show obvious jagged high-contrast edges at 125% and 175%.
- Custom font fallback still measures and draws Chinese, ASCII, digits, and punctuation without clipping.

## Notification Window

- `WM_DPICHANGED` resizes to the newly measured text size, not just the suggested old size.
- Long notification text ellipsizes cleanly after crossing monitors.
- Border weight remains visible on 150% and 200% screens.

## Settings Preview

- Compare each `skins/*/theme.json` preview with the real candidate window.
- Check color, selected row/card weight, rounded corners, chip colors, and horizontal spacing.
- Prefer the real candidate window when preview and runtime differ; the preview is only an approximation.
