"""Generate and verify the twenty coordinated candidate palettes (stdlib only)."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# Ten light and ten dark palettes; different paper/ink bases, not just accent swaps.
PALETTES = [
    ("porcelain-jade", "瓷白青玉", "#FAFCF9", "#E0F0E8", "#237355", "#203A30", "#51665D", False),
    ("linen-oat", "亚麻燕麦", "#FAF7F0", "#EDE3CE", "#806229", "#3B3325", "#6A604F", False),
    ("sakura-mist", "樱花薄雾", "#FFF8FA", "#F6E2EB", "#A44370", "#442C39", "#735360", False),
    ("glacier-blue", "冰川浅蓝", "#F5FAFF", "#DDEBF8", "#286A9F", "#243A4C", "#506578", False),
    ("lilac-letter", "丁香信笺", "#FAF8FF", "#EAE3F7", "#7652A2", "#382E4A", "#635775", False),
    ("peach-cream", "蜜桃奶油", "#FFF9F3", "#F9E5D5", "#A35630", "#493326", "#765D4E", False),
    ("sage-garden", "鼠尾草园", "#F5F9F3", "#E2ECD9", "#527436", "#303D28", "#58664E", False),
    ("lemon-paper", "柠檬手札", "#FFFDF2", "#F2EECF", "#80711C", "#403D25", "#686348", False),
    ("aqua-breeze", "水色微风", "#F3FBFB", "#DAEFEE", "#267571", "#263F40", "#4D6868", False),
    ("silver-cloud", "银灰云朵", "#F8F9FB", "#E4E8F0", "#546986", "#303947", "#596575", False),
    ("obsidian-jade", "曜石青玉", "#191F1D", "#293F35", "#82C9A6", "#E5F1E9", "#ABBEB1", True),
    ("midnight-tide", "午夜潮汐", "#171E2B", "#263A51", "#8CBFE8", "#E4ECF6", "#ACBDCF", True),
    ("plum-velvet", "梅紫绒夜", "#251C2A", "#44304B", "#D7A0DE", "#F2E6F4", "#C9B1CE", True),
    ("cocoa-ember", "可可余烬", "#261F1B", "#46372B", "#DFB184", "#F3EBE1", "#C6B5A4", True),
    ("pine-shadow", "松林暗影", "#17231F", "#294236", "#95C9A4", "#E2F0E5", "#ADC5B5", True),
    ("ruby-evening", "红宝晚霞", "#2A1B20", "#4C2D39", "#E4A1B4", "#F7E5EC", "#CDB0BB", True),
    ("indigo-orbit", "靛蓝星轨", "#1C1E31", "#30385A", "#ADB9F1", "#E9EBFC", "#B8BEDD", True),
    ("copper-night", "铜色夜幕", "#28201C", "#48372F", "#E4B092", "#F8EADF", "#CDB6A7", True),
    ("petrol-deep", "深海蓝绿", "#142427", "#244147", "#83CAD1", "#E0F2F4", "#A8C4C8", True),
    ("graphite-gold", "石墨微金", "#22221F", "#3D3C30", "#D5CA91", "#F0F0E6", "#C1C1AD", True),
]


def rgb(value):
    return tuple(int(value[i:i + 2], 16) for i in (1, 3, 5))


def mix(a, b, weight):
    return "#" + "".join(f"{round(x * (1 - weight) + y * weight):02X}" for x, y in zip(rgb(a), rgb(b)))


def contrast(a, b):
    def lum(value):
        channels = [v / 255 for v in rgb(value)]
        linear = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in channels]
        return sum(x * w for x, w in zip(linear, (0.2126, 0.7152, 0.0722)))
    x, y = sorted((lum(a), lum(b)))
    return (y + 0.05) / (x + 0.05)


def themes():
    for key, name, bg, selected, accent, text, muted, dark in PALETTES:
        hover = mix(bg, selected, 0.55)
        pressed = mix(selected, accent, 0.08)
        theme = dict(
            name=name, material="solid", layout="classic", font_size=16,
            window_bg=bg, header_bg=bg, border=mix(bg, muted, 0.30), divider=mix(bg, muted, 0.16),
            item_bg=bg, item_border=bg, hover_bg=hover, hover_border=hover,
            selected_bg=selected, selected_border=accent, pressed_bg=pressed, pressed_border=pressed,
            text=text, muted_text=muted, selected_text=text, selected_muted_text=muted,
            badge_bg=bg, badge_border=bg, badge_text=muted,
            chip_bg=hover, chip_border=hover, chip_text=muted,
            chip_active_bg=selected, chip_active_border=selected, chip_active_text=text,
            selected_outline=accent, selected_accent_width=2, selected_ring_opacity=0,
            selected_indicator="left_bar", border_opacity=0.8, divider_opacity=0.12,
            shadow_enabled=True, shadow_opacity=0.10 if dark else 0.07, shadow_size=9,
            font_weight=400, selected_font_weight=600, label_font_weight=400, chip_font_weight=400,
            corner_radius=12, header_corner_radius=8, row_corner_radius=7, badge_corner_radius=4,
            outer_pad_x=8, outer_pad_y=7, header_pad_x=10, header_pad_y=5, header_gap=5,
            item_gap=3, item_pad_x=10, item_pad_y=6, label_width=22, label_gap=8, comment_gap=5,
            min_width=220, preferred_width=340, max_width=600,
            min_horizontal_card_width=88, max_horizontal_card_width=200,
            animations_enabled=True, show_animation_ms=0, selection_animation_ms=65,
            hover_animation_ms=45, press_animation_ms=24, page_animation_ms=0,
        )
        # Keep all small text >= 4.5:1, including every interactive surface.
        surfaces = [bg, selected, hover, pressed]
        if min(contrast(muted, surface) for surface in surfaces) < 4.5:
            muted = mix(muted, text, 0.35)
            for field in ("muted_text", "selected_muted_text", "badge_text", "chip_text"):
                theme[field] = muted
        for fg in (text, muted):
            for surface in surfaces:
                assert contrast(fg, surface) >= 4.5, (key, fg, surface, contrast(fg, surface))
        yield key, theme


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for key, theme in themes():
        path = ROOT / "skins" / key / "theme.json"
        if args.check:
            assert json.loads(path.read_text(encoding="utf-8")) == theme, path
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(theme, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("20 candidate skins verified: normal, hover, selected and pressed text >= 4.5:1")
