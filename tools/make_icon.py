from __future__ import annotations

from pathlib import Path

from PIL import Image


def _pad_to_square_rgba(im: Image.Image) -> Image.Image:
    im = im.convert("RGBA")
    w, h = im.size
    side = max(w, h)
    if (w, h) == (side, side):
        return im
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(im, ((side - w) // 2, (side - h) // 2))
    return canvas


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    src = repo_root / "assets" / "e__CODE_code1____1_img11.png"
    if not src.exists():
        # fallback: user dropped it in repo root as img11.png
        alt = repo_root / "img11.png"
        if alt.exists():
            src = alt
        else:
            raise FileNotFoundError(f"找不到源图片：{src} 或 {alt}")

    out_dir = repo_root / "icons"
    out_dir.mkdir(parents=True, exist_ok=True)

    im = _pad_to_square_rgba(Image.open(src))

    png_sizes = [16, 24, 32, 40, 48, 64, 96, 128, 256, 512]
    for s in png_sizes:
        im.resize((s, s), Image.Resampling.LANCZOS).save(out_dir / f"app_icon_{s}.png")

    # Windows shell/Explorer typically uses up to 256 in ICO
    ico_sizes = [(s, s) for s in [16, 24, 32, 40, 48, 64, 96, 128, 256]]
    im.save(out_dir / "app_icon.ico", format="ICO", sizes=ico_sizes)

    # Extra large PNG for platforms that want it
    im.resize((1024, 1024), Image.Resampling.LANCZOS).save(out_dir / "app_icon_1024.png")

    print(f"OK: {src} -> {out_dir / 'app_icon.ico'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

