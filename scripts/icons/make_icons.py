# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow>=10"]
# ///
"""Draw Longhand's source icon and tray template icon.

The source icon feeds `bun tauri icon`, which generates every platform size.
The tray icon is a macOS template image: black on transparent, recoloured by
the menu bar for light and dark appearance.
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw


def draw_mark(draw: ImageDraw.ImageDraw, size: int, ink) -> None:
    """A capital L with a pen dot past the end of its foot."""
    s = size / 100
    stroke = round(11 * s)
    # Upright.
    draw.rounded_rectangle([30 * s, 18 * s, 30 * s + stroke, 74 * s], radius=stroke / 2, fill=ink)
    # Foot, then the dot.
    draw.rounded_rectangle([30 * s, 74 * s - stroke, 60 * s, 74 * s], radius=stroke / 2, fill=ink)
    draw.ellipse([66 * s, 74 * s - stroke, 66 * s + stroke, 74 * s], fill=ink)


def source_icon(path: Path) -> None:
    size = 1024
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    inset = round(size * 0.09)
    draw.rounded_rectangle(
        [inset, inset, size - inset, size - inset], radius=round(size * 0.2), fill=(28, 36, 52, 255)
    )
    draw_mark(draw, size, (242, 236, 222, 255))
    img.save(path)


def tray_icon(path: Path) -> None:
    size = 44  # 22 pt at 2x, the macOS menu-bar height.
    scale = 8  # supersample, then downsample for smooth edges
    big = Image.new("RGBA", (size * scale, size * scale), (0, 0, 0, 0))
    draw_mark(ImageDraw.Draw(big), size * scale, (0, 0, 0, 255))
    big.resize((size, size), Image.LANCZOS).save(path)


if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    source_icon(out / "app-icon.png")
    tray_icon(out / "tray.png")
