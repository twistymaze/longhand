# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow==11.3.0", "resvg-py==0.5.0"]
# ///
"""Render the shared SVG mark as the desktop source and tray template.

The source PNG feeds `npm run tauri -- icon` for macOS and Windows. The tray
stays black on transparent so macOS can set its appearance.
"""

import io
import sys
from pathlib import Path
import xml.etree.ElementTree as ET

from PIL import Image
import resvg_py

MARK = Path(__file__).resolve().parents[2] / "src/assets/longhand-mark.svg"
ET.register_namespace("", "http://www.w3.org/2000/svg")


def mark_geometry() -> str:
    root = ET.fromstring(MARK.read_text())
    return "".join(ET.tostring(child, encoding="unicode") for child in root)


def render(svg: str, size: int) -> Image.Image:
    png = resvg_py.svg_to_bytes(
        svg_string=svg, width=size, height=size, skip_system_fonts=True
    )
    return Image.open(io.BytesIO(png)).convert("RGBA")


def source_icon(path: Path) -> None:
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
      <defs>
        <linearGradient id="ink" gradientUnits="userSpaceOnUse" x1="0" y1="18" x2="0" y2="74">
          <stop stop-color="#f7f1e3" />
          <stop offset="1" stop-color="#c6bfae" />
        </linearGradient>
      </defs>
      <rect x="9" y="9" width="82" height="82" rx="20" fill="#1c2434" />
      <g fill="url(#ink)">{mark_geometry()}</g>
    </svg>'''
    render(svg, 1024).save(path)


def tray_icon(path: Path) -> None:
    # Enlarge the visible mark by 25% in the same 22 pt slot.
    # The viewBox center matches the mark's bounds (53.5, 46).
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="13.5 6 80 80">
      <g fill="black">{mark_geometry()}</g>
    </svg>'''
    render(svg, 44 * 8).resize((44, 44), Image.Resampling.LANCZOS).save(path)


if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    source_icon(out / "app-icon.png")
    tray_icon(out / "tray.png")
