"""Writes the HEIC inputs for the import cases (import/heic-*).

The corpus generator can't encode HEVC itself, so these files are made once with this script and
committed; gen-corpus copies them into the corpus. The pictures are drawn here from fixed formulas
(no photographs), so the files carry no third-party content. They are encoded by libheif 1.23.4
with the x265 4.3 HEVC encoder, through pillow-heif 1.8.0 (`pip install pillow-heif==1.8.0`).
Re-running the script with other versions can give different bytes; the committed files are the
reference.
"""

import colorsys
import math
from pathlib import Path

import pillow_heif
from PIL import Image

HERE = Path(__file__).parent


def picture(w, h, alpha=False):
    img = Image.new("RGBA" if alpha else "RGB", (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            t, v = x / (w - 1), y / (h - 1)
            if y >= h - h // 8:
                g = round(t * 255)
                rgb = (g, g, g)
            else:
                r, g, b = colorsys.hsv_to_rgb(t, 0.25 + 0.75 * (1 - v * 0.5), 0.1 + 0.9 * (1 - v))
                rgb = (round(r * 255), round(g * 255), round(b * 255))
            if alpha:
                d = math.hypot(x + 0.5 - w / 2, y + 0.5 - h / 2) / (min(w, h) / 2)
                a = max(0, min(255, round((1.2 - d) * 255)))
                px[x, y] = rgb + (a,)
            else:
                px[x, y] = rgb
    return img


def noise(w, h, seed):
    """Per-pixel pseudo-random colors, so chroma changes at every sample (for fitting the
    upsampling Apple's decoder uses)."""
    img = Image.new("RGB", (w, h))
    px = img.load()
    state = seed
    for y in range(h):
        for x in range(w):
            rgb = []
            for _ in range(3):
                state = (state * 1103515245 + 12345) & 0x7FFFFFFF
                rgb.append(state >> 23)
            px[x, y] = tuple(rgb)
    return img


def main():
    cases = [
        ("heic-420.heic", picture(64, 48), {"quality": 90, "chroma": 420}),
        ("heic-444.heic", picture(64, 48), {"quality": 90, "chroma": 444}),
        ("heic-rgba.heic", picture(64, 48, alpha=True), {"quality": 90, "chroma": 420}),
        ("heic-odd-420.heic", picture(47, 31), {"quality": 90, "chroma": 420}),
        ("heic-noise-420.heic", noise(64, 64, 1), {"quality": 100, "chroma": 420}),
        ("heic-noise-444.heic", noise(64, 64, 2), {"quality": 100, "chroma": 444}),
    ]
    for name, img, options in cases:
        heif = pillow_heif.from_pillow(img)
        heif.save(HERE / name, **options)
        print(name, (HERE / name).stat().st_size, "bytes")


if __name__ == "__main__":
    main()
