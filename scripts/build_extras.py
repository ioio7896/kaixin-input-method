"""Optional skin checks and local diagnostics for the build command."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parents[1]
_HEX_COLOR_RE = re.compile(r"^#?[0-9a-fA-F]{6}$")


def _is_color_like(value: object) -> bool:
    return isinstance(value, str) and _HEX_COLOR_RE.match(value.strip()) is not None


def _parse_theme_color(value: object) -> tuple[int, int, int] | None:
    if not _is_color_like(value):
        return None
    text = str(value).strip().lstrip("#")
    return (int(text[0:2], 16), int(text[2:4], 16), int(text[4:6], 16))


def _relative_luminance(rgb: tuple[int, int, int]) -> float:
    def channel(v: int) -> float:
        s = v / 255.0
        return s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4

    r, g, b = rgb
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def _contrast_ratio(a: tuple[int, int, int], b: tuple[int, int, int]) -> float:
    la = _relative_luminance(a)
    lb = _relative_luminance(b)
    hi, lo = (la, lb) if la >= lb else (lb, la)
    return (hi + 0.05) / (lo + 0.05)


def _validate_theme_json(path: Path) -> list[str]:
    theme_name = f"{path.parent.name}/{path.name}"
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        raise RuntimeError(f"Could not read theme file: {path} ({e})") from e
    except OSError as e:
        raise RuntimeError(f"Could not read theme file: {path} ({e})") from e

    if not isinstance(data, dict):
        raise RuntimeError(f"Theme root must be a JSON object: {path}")

    warnings: list[str] = []
    required = ("name", "window_bg", "header_bg", "text", "selected_bg", "selected_text")
    missing = [k for k in required if k not in data]
    if missing:
        warnings.append(f"{theme_name}: missing required keys: {', '.join(missing)}")

    color_keys = [
        "window_bg",
        "window_bg_to",
        "header_bg",
        "header_bg_to",
        "border",
        "divider",
        "item_bg",
        "item_border",
        "hover_bg",
        "hover_border",
        "selected_bg",
        "selected_border",
        "pressed_bg",
        "pressed_border",
        "text",
        "muted_text",
        "selected_text",
        "selected_muted_text",
        "badge_bg",
        "badge_border",
        "badge_text",
        "chip_bg",
        "chip_border",
        "chip_text",
        "chip_active_bg",
        "chip_active_border",
        "chip_active_text",
        "selected_outline",
    ]
    for key in color_keys:
        if key in data and data[key] is not None and not _is_color_like(data[key]):
            warnings.append(f"{theme_name}: field {key} is not a valid color (#RRGGBB expected)")

    def color_for(key: str, fallback: str | None = None) -> tuple[int, int, int] | None:
        return _parse_theme_color(data.get(key)) or (
            _parse_theme_color(data.get(fallback)) if fallback else None
        )

    contrast_checks = [
        ("text", "item_bg", "window_bg", 4.2),
        ("muted_text", "item_bg", "window_bg", 3.2),
        ("selected_text", "selected_bg", None, 4.2),
        ("selected_muted_text", "selected_bg", None, 3.2),
        ("badge_text", "badge_bg", None, 3.6),
        ("chip_text", "chip_bg", None, 3.6),
        ("chip_active_text", "chip_active_bg", None, 4.2),
    ]
    for fg_key, bg_key, bg_fallback, min_ratio in contrast_checks:
        fg = color_for(fg_key)
        bg = color_for(bg_key, bg_fallback)
        if not fg or not bg:
            continue
        ratio = _contrast_ratio(fg, bg)
        if ratio < min_ratio:
            warnings.append(
                f"{theme_name}: {fg_key} on {bg_key} contrast is {ratio:.2f}:1 "
                f"(recommended >= {min_ratio:.1f}:1)"
            )

    for key in ("shadow_enabled", "animations_enabled"):
        if key in data and not isinstance(data[key], bool):
            warnings.append(f"{theme_name}: {key} should be boolean")
    numeric_ranges = {
        "selected_accent_width": (0, 8),
        "selected_ring_opacity": (0.0, 1.0),
        "border_opacity": (0.0, 1.0),
        "divider_opacity": (0.0, 1.0),
        "shadow_opacity": (0.0, 1.0),
        "shadow_size": (0, 24),
        "font_weight": (300, 700),
        "selected_font_weight": (400, 800),
        "label_font_weight": (400, 800),
        "chip_font_weight": (350, 700),
        "show_animation_ms": (0, 240),
        "selection_animation_ms": (0, 240),
        "hover_animation_ms": (0, 240),
        "press_animation_ms": (0, 240),
        "page_animation_ms": (0, 240),
    }
    for key, (lo, hi) in numeric_ranges.items():
        if key not in data:
            continue
        value = data[key]
        if (
            not isinstance(value, (int, float))
            or isinstance(value, bool)
            or value < lo
            or value > hi
        ):
            warnings.append(f"{theme_name}: {key} should be numeric in range {lo}..{hi}")
    if "selected_indicator" in data and data["selected_indicator"] not in (
        "left_bar",
        "bottom_bar",
        "outline",
        "none",
    ):
        warnings.append(
            f"{theme_name}: selected_indicator should be left_bar, bottom_bar, outline, or none"
        )

    return warnings


def validate_skins(repo: Path) -> list[str]:
    skins_root = repo / "skins"
    if not skins_root.is_dir():
        raise FileNotFoundError(f"skins directory not found: {skins_root}")
    theme_files = sorted(skins_root.glob("*/theme.json"), key=lambda p: p.as_posix().lower())
    if not theme_files:
        raise FileNotFoundError(f"no skin theme.json found: {skins_root}")

    warnings: list[str] = []
    for theme in theme_files:
        warnings.extend(_validate_theme_json(theme))
    print(f"Skin validation passed: {len(theme_files)} themes")
    return warnings


def runtime_log_roots() -> list[Path]:
    roots: list[Path] = []
    local_app_data = os.environ.get("LOCALAPPDATA")
    if local_app_data:
        base = Path(local_app_data)
        roots.extend(
            [
                base / "kaixin" / "logs",
                base / "kaixin",
            ]
        )
    program_data = os.environ.get("PROGRAMDATA")
    if program_data:
        roots.append(Path(program_data) / "kaixin")

    unique: list[Path] = []
    seen: set[str] = set()
    for root in roots:
        key = str(root).lower()
        if key not in seen:
            seen.add(key)
            unique.append(root)
    return unique


def known_runtime_logs() -> list[str]:
    current_logs = [
        "tsf.log",
        "engine.log",
        "tray.log",
        "clipboard.log",
    ]
    rotated_logs: list[str] = []
    for name in current_logs:
        stem = name.rsplit(".", 1)[0]
        rotated_logs.append(f"{stem}.previous.log")
        rotated_logs.extend(f"{stem}.previous.{index}.log" for index in range(2, 6))

    return [
        *current_logs,
        *rotated_logs,
        "compatibility.log",
        "install_user.log",
        "install_machine.log",
        "install_language_list.log",
        "uninstall_user.log",
        "uninstall_machine.log",
        "register.log",
        "tip_registration.log",
        "tip_registration_user.log",
    ]


def collect_runtime_logs(repo: Path) -> list[Path]:
    dest_root = repo / "dist" / "runtime-logs"

    copied: list[Path] = []
    used_labels: set[str] = set()
    for source_root in runtime_log_roots():
        if not source_root.is_dir():
            continue
        label = source_root.name
        if label.lower() in used_labels:
            parent_label = re.sub(r"[^0-9A-Za-z_.-]+", "_", source_root.parent.name) or "root"
            label = f"{parent_label}-{source_root.name}"
        used_labels.add(label.lower())
        source_dest_root = dest_root / label
        source_dest_root.mkdir(parents=True, exist_ok=True)

        for name in known_runtime_logs():
            src = source_root / name
            if not src.is_file():
                continue
            dest = source_dest_root / name
            shutil.copy2(src, dest)
            copied.append(dest)
    return copied


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("skins", "logs"))
    if parser.parse_args().action == "skins":
        for warning in validate_skins(ROOT):
            print(f"Skin warning: {warning}")
    else:
        copied = collect_runtime_logs(ROOT)
        print(f"Collected {len(copied)} runtime logs in {ROOT / 'dist/runtime-logs'}")


if __name__ == "__main__":
    main()
