#!/usr/bin/env python3
"""Generate the Elyra Workspace app icon from the Elyra Conductor icon.

Elyra Workspace uses the same Lyra constellation icon as Elyra Conductor,
recolored from orange to yellow (#fbd22d). Requires Pillow and macOS `iconutil`.

    python3 scripts/recolor-icon.py ~/Code/elyra-conductor/src-tauri/icons/icon.icns
"""

import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image

# Orange (hue 27°) -> yellow (hue 48°), on PIL's 0-255 hue scale.
HUE_SHIFT = round(21 / 360 * 255)
SATURATION = 1.08
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets", "icon")


def recolor(src: str, dst: str) -> None:
    image = Image.open(src).convert("RGBA")
    alpha = image.getchannel("A")
    hue, sat, val = image.convert("RGB").convert("HSV").split()
    hue = hue.point(lambda x: (x + HUE_SHIFT) % 256)
    sat = sat.point(lambda x: min(255, int(x * SATURATION)))
    out = Image.merge("HSV", (hue, sat, val)).convert("RGB")
    out.putalpha(alpha)
    out.save(dst)


def main() -> None:
    source = os.path.expanduser(sys.argv[1] if len(sys.argv) > 1 else
                                "~/Code/elyra-conductor/src-tauri/icons/icon.icns")
    with tempfile.TemporaryDirectory() as tmp:
        src_set = os.path.join(tmp, "src.iconset")
        dst_set = os.path.join(tmp, "icon.iconset")
        subprocess.run(["iconutil", "-c", "iconset", "-o", src_set, source], check=True)
        os.makedirs(dst_set)
        for name in os.listdir(src_set):
            if name.endswith(".png"):
                recolor(os.path.join(src_set, name), os.path.join(dst_set, name))
        os.makedirs(OUT, exist_ok=True)
        subprocess.run(["iconutil", "-c", "icns", "-o", os.path.join(OUT, "icon.icns"), dst_set],
                       check=True)
        shutil.copy(os.path.join(dst_set, "icon_512x512@2x.png"), os.path.join(OUT, "icon.png"))
        shutil.copy(os.path.join(dst_set, "icon_128x128@2x.png"), os.path.join(OUT, "icon-256.png"))
    print(f"wrote {OUT}/icon.icns, icon.png, icon-256.png")


if __name__ == "__main__":
    main()
