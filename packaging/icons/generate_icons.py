#!/usr/bin/env python3
"""Render the Prompt F app icon to every PNG size and icon.ico.

Geometry and colors match ferrum-icon.svg (32 px and up) and ferrum-icon-small.svg
(16 and 32 px hint). Needs Pillow. Run from anywhere:
    python3 packaging/icons/generate_icons.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
ASSETS = HERE.parent.parent / "assets"
SS = 2048  # supersampled canvas; the masters are drawn on a 1024 grid


def hexrgb(value):
    return tuple(int(value[i : i + 2], 16) for i in (1, 3, 5))


def gradient(top, bottom, horizontal=False):
    a, b = hexrgb(top), hexrgb(bottom)
    line = Image.new("RGB", (SS, 1) if horizontal else (1, SS))
    for i in range(SS):
        t = i / (SS - 1)
        c = tuple(round(a[k] + (b[k] - a[k]) * t) for k in range(3))
        line.putpixel((i, 0) if horizontal else (0, i), c)
    return line.resize((SS, SS))


def mask_poly(points):
    m = Image.new("L", (SS, SS), 0)
    ImageDraw.Draw(m).polygon([(x * SS / 1024, y * SS / 1024) for x, y in points], fill=255)
    return m


def mask_rrect(x, y, w, h, r):
    s = SS / 1024
    m = Image.new("L", (SS, SS), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        (x * s, y * s, (x + w) * s, (y + h) * s), r * s, fill=255
    )
    return m


def master():
    img = Image.new("RGBA", (SS, SS), (0, 0, 0, 0))
    img.paste(gradient("#2B2B30", "#131315"), mask=mask_rrect(100, 100, 824, 824, 186))
    # 1-unit hairline on the tile edge, white at 10%.
    edge = mask_rrect(100, 100, 824, 824, 186)
    inner = mask_rrect(104, 104, 816, 816, 182)
    ring = Image.composite(Image.new("L", (SS, SS), 0), edge, inner).point(lambda v: v // 10)
    img.paste(Image.new("RGB", (SS, SS), "#FFFFFF"), mask=ring)
    img.paste(
        gradient("#F8F6F1", "#BDBAB3"),
        mask=mask_poly([(304, 220), (806, 220), (760, 352), (372, 352), (372, 804), (240, 804), (240, 284)]),
    )
    hot = gradient("#FF6A2B", "#FFA766", horizontal=True)
    img.paste(hot, mask=mask_poly([(440, 432), (528, 432), (646, 544), (528, 656), (440, 656), (558, 544)]))
    img.paste(hot, mask=mask_poly([(610, 736), (806, 736), (806, 804), (610, 804)]))
    return img


def small():
    img = Image.new("RGBA", (SS, SS), (0, 0, 0, 0))
    img.paste(Image.new("RGB", (SS, SS), "#1C1C1F"), mask=mask_rrect(48, 48, 928, 928, 208))
    img.paste(
        Image.new("RGB", (SS, SS), "#F2F0EB"),
        mask=mask_poly([(170, 170), (860, 170), (860, 330), (340, 330), (340, 854), (170, 854)]),
    )
    ember = Image.new("RGB", (SS, SS), "#FF8A4C")
    img.paste(ember, mask=mask_poly([(410, 410), (530, 410), (680, 556), (530, 702), (410, 702), (560, 556)]))
    img.paste(ember, mask=mask_poly([(610, 760), (860, 760), (860, 854), (610, 854)]))
    return img


def main():
    large, tiny = master(), small()
    rendered = {}
    for size in (16, 32, 64, 128, 256, 512, 1024):
        source = tiny if size <= 32 else large
        rendered[size] = source.resize((size, size), Image.LANCZOS)
        rendered[size].save(HERE / f"icon_{size}.png", optimize=True)
    rendered[256].save(ASSETS / "icon.png", optimize=True)
    rendered[256].save(
        HERE / "icon.ico",
        sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256)],
        append_images=[rendered[s] for s in (16, 32, 64, 128)],
    )


if __name__ == "__main__":
    main()
