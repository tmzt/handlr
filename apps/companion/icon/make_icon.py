#!/usr/bin/env python3
"""Regenerate handlr's Android launcher icon from the Gemini source image.

    python3 -I apps/companion/icon/make_icon.py [--source PATH] [--hue DEG]
                                               [--background RRGGBB]

The source (2752x1536) shows the icon twice; the LEFT copy, on white, is used.
The rounded navy square is found by flood-filling the white surround, the
line art is unmixed from the navy (alpha from brightness above the navy, then
the colour solved from `p = a*C + (1-a)*navy`), its hue is rotated, and every
output is drawn from those two layers: a flat navy shape, drawn
mathematically so no white JPEG fringe survives, and the line art on top.

Writes, next to this script:
  icon-1024.png                 master, the source of truth
  icon-512.png                  Play-store size
  foreground-1024.png           the dog alone on transparency (unpadded)
  preview.png                   contact sheet of the masters and every size
  android/res/mipmap-*/ic_launcher{,_round,_foreground}.png
  android/res/mipmap-anydpi-v26/ic_launcher{,_round}.xml
  android/res/values/ic_launcher_background.xml
"""

import argparse
import colorsys
import os

import numpy as np
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SOURCE = "/Users/tmeade/Projects/Handlr/Gemini_Generated_Image_aq11hwaq11hwaq11.jpeg"
DEFAULT_HUE = 110.0  # degrees added to the line art's HSV hue
DEFAULT_BACKGROUND = "0F0F26"  # the source's navy (0A1025), nudged towards purple

MASTER = 1024
# Corner radius of the rounded square as a fraction of its side, measured
# from the source (radius ~245 px on a ~1100 px square).
CORNER = 0.223
SS = 4  # supersampling for the drawn shapes

DENSITIES = {"mdpi": 1.0, "hdpi": 1.5, "xhdpi": 2.0, "xxhdpi": 3.0, "xxxhdpi": 4.0}
LEGACY_DP = 48
ADAPTIVE_DP = 108
SAFE_DP = 66  # diameter of the adaptive icon's guaranteed-visible circle
# Legacy icons: the shape fills 44 of the 48 dp, as Material's keylines do.
LEGACY_FILL = 44 / 48


def find_square(rgb):
    """Bounding box (x0, y0, x1, y1) of the left icon's rounded square."""
    left = rgb.crop((0, 0, rgb.width // 2, rgb.height)).convert("L")
    lum = np.asarray(left)
    mask = Image.fromarray(np.where(lum > 128, 255, 0).astype("uint8"), "L").copy()
    ImageDraw.floodfill(mask, (0, 0), 128)
    inside = np.array(mask) != 128
    ys, xs = np.nonzero(inside)
    return xs.min(), ys.min(), xs.max() + 1, ys.max() + 1


def line_art(rgb, box, hue_deg):
    """The hue-rotated line art as straight-alpha RGBA, MASTER x MASTER."""
    crop = rgb.crop(box).resize((MASTER, MASTER), Image.LANCZOS)
    p = np.asarray(crop).astype(np.float64)
    # The navy, from a flat patch in the upper middle of the square.
    navy = np.median(p[40:140, 300:700].reshape(-1, 3), axis=0)
    # Alpha from the brightest channel above the navy's; full-strength line
    # colours peak around 0.86 in value. A small floor eats JPEG noise.
    full = 219.0
    floor = 8.0
    v = p.max(axis=2)
    a = np.clip((v - navy.max() - floor) / (full - navy.max() - floor), 0.0, 1.0)
    # Nothing near the edge of the square is line art (the dog sits well
    # inside); this drops the white anti-aliased rim.
    edge = rounded_mask(MASTER, inset=24)
    a *= np.asarray(edge, dtype=np.float64) / 255.0
    safe = np.maximum(a, 1e-3)[..., None]
    c = np.clip((p - (1.0 - a[..., None]) * navy) / safe, 0.0, 255.0)
    c = rotate_hue(c, hue_deg)
    out = np.dstack([c, a * 255.0]).round().astype(np.uint8)
    return Image.fromarray(out, "RGBA"), navy


def rotate_hue(c, deg):
    """Add `deg` to the HSV hue of every pixel, keeping saturation and value."""
    flat = c.reshape(-1, 3) / 255.0
    r, g, b = flat[:, 0], flat[:, 1], flat[:, 2]
    mx = flat.max(axis=1)
    mn = flat.min(axis=1)
    d = mx - mn
    h = np.zeros_like(mx)
    nz = d > 1e-9
    rm = nz & (mx == r)
    gm = nz & (mx == g) & ~rm
    bm = nz & ~rm & ~gm
    h[rm] = ((g - b)[rm] / d[rm]) % 6.0
    h[gm] = (b - r)[gm] / d[gm] + 2.0
    h[bm] = (r - g)[bm] / d[bm] + 4.0
    h = (h + deg / 60.0) % 6.0
    s = np.where(mx > 0, d / np.maximum(mx, 1e-9), 0.0)
    v = mx
    i = np.floor(h).astype(int) % 6
    f = h - np.floor(h)
    pp = v * (1 - s)
    q = v * (1 - s * f)
    t = v * (1 - s * (1 - f))
    choices = [
        np.stack([v, t, pp], 1),
        np.stack([q, v, pp], 1),
        np.stack([pp, v, t], 1),
        np.stack([pp, q, v], 1),
        np.stack([t, pp, v], 1),
        np.stack([v, pp, q], 1),
    ]
    out = np.zeros_like(flat)
    for k in range(6):
        out[i == k] = choices[k][i == k]
    return (out * 255.0).reshape(c.shape)


def rounded_mask(size, inset=0, corner=CORNER):
    """Anti-aliased rounded-square mask (L) of side `size`."""
    big = Image.new("L", (size * SS, size * SS), 0)
    r = corner * (size - 2 * inset) * SS
    ImageDraw.Draw(big).rounded_rectangle(
        (inset * SS, inset * SS, (size - inset) * SS - 1, (size - inset) * SS - 1),
        radius=r,
        fill=255,
    )
    return big.resize((size, size), Image.LANCZOS)


def circle_mask(size):
    big = Image.new("L", (size * SS, size * SS), 0)
    ImageDraw.Draw(big).ellipse((0, 0, size * SS - 1, size * SS - 1), fill=255)
    return big.resize((size, size), Image.LANCZOS)


def flat(size, rgb, mask):
    img = Image.new("RGBA", (size, size), rgb + (0,))
    img.putalpha(mask)
    return img


def over(base, top):
    out = base.copy()
    out.alpha_composite(top)
    return out


def content_box(art):
    a = np.asarray(art.getchannel("A"))
    ys, xs = np.nonzero(a > 4)
    return xs.min(), ys.min(), xs.max() + 1, ys.max() + 1


def place(art, canvas, scale_px, centre):
    """The dog (cropped to its content) scaled so its farthest pixel lies
    `scale_px` from its centre, pasted centred at `centre` on a transparent
    canvas of side `canvas`. Drawn at MASTER-ish resolution, downscaled after."""
    x0, y0, x1, y1 = content_box(art)
    dog = art.crop((x0, y0, x1, y1))
    a = np.asarray(dog.getchannel("A")) > 4
    ys, xs = np.nonzero(a)
    cx, cy = (x1 - x0) / 2.0, (y1 - y0) / 2.0
    radius = np.sqrt((xs + 0.5 - cx) ** 2 + (ys + 0.5 - cy) ** 2).max()
    k = scale_px / radius
    w, h = max(1, round(dog.width * k)), max(1, round(dog.height * k))
    dog = premul_resize(dog, (w, h))
    out = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    out.alpha_composite(dog, (round(centre - w / 2), round(centre - h / 2)))
    return out


def premul_resize(img, size):
    """Lanczos resize in premultiplied alpha, so transparent pixels' colour
    does not bleed into edges."""
    arr = np.asarray(img).astype(np.float64)
    a = arr[..., 3:4] / 255.0
    pre = np.dstack([arr[..., :3] * a, arr[..., 3:4]])
    chans = [
        np.asarray(Image.fromarray(pre[..., k].astype(np.float32), "F").resize(size, Image.LANCZOS))
        for k in range(4)
    ]
    pre = np.stack(chans, axis=-1)
    al = np.clip(pre[..., 3], 0, 255)
    rgb = np.where(al[..., None] > 0, pre[..., :3] / np.maximum(al[..., None], 1e-6) * 255.0, 0)
    out = np.dstack([np.clip(rgb, 0, 255), al]).round().astype(np.uint8)
    return Image.fromarray(out, "RGBA")


def write_png(img, *parts):
    path = os.path.join(HERE, *parts)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.save(path, optimize=True)
    return path


def write_text(text, *parts):
    path = os.path.join(HERE, *parts)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


ADAPTIVE_XML = """<?xml version="1.0" encoding="utf-8"?>
<!-- GENERATED by apps/companion/icon/make_icon.py. Do not edit. -->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@color/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
    <monochrome android:drawable="@mipmap/ic_launcher_foreground" />
</adaptive-icon>
"""

COLOR_XML = """<?xml version="1.0" encoding="utf-8"?>
<!-- GENERATED by apps/companion/icon/make_icon.py. Do not edit. -->
<resources>
    <color name="ic_launcher_background">#{hex}</color>
</resources>
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--source", default=DEFAULT_SOURCE)
    ap.add_argument("--hue", type=float, default=DEFAULT_HUE)
    ap.add_argument("--background", default=DEFAULT_BACKGROUND, help="RRGGBB, or 'source'")
    args = ap.parse_args()

    rgb = Image.open(args.source).convert("RGB")
    box = find_square(rgb)
    art, navy = line_art(rgb, box, args.hue)
    if args.background == "source":
        bg = tuple(int(round(x)) for x in navy)
    else:
        bg = tuple(int(args.background[i : i + 2], 16) for i in (0, 2, 4))
    hexbg = "%02X%02X%02X" % bg
    print(f"square {box}, source navy {tuple(int(round(x)) for x in navy)}, "
          f"background #{hexbg}, hue +{args.hue:g}")

    # Masters: the full icon as in the source composition, and the dog alone.
    master = over(flat(MASTER, bg, rounded_mask(MASTER)), art)
    write_png(master, "icon-1024.png")
    write_png(master.resize((512, 512), Image.LANCZOS), "icon-512.png")
    x0, y0, x1, y1 = content_box(art)
    write_png(art.crop((x0, y0, x1, y1)), "foreground-1024.png")

    # Legacy icons, drawn at 4x of xxxhdpi and downscaled. The shape fills
    # LEGACY_FILL of the canvas; the dog keeps its place within the square.
    big = 4 * 192
    shape = round(big * LEGACY_FILL)
    pad = (big - shape) // 2
    square_icon = premul_resize(master, (shape, shape))
    legacy = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    legacy.alpha_composite(square_icon, (pad, pad))
    # Round: a navy circle, the dog inside the circle's inscribed safe area.
    round_icon = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    round_icon.alpha_composite(flat(shape, bg, circle_mask(shape)), (pad, pad))
    round_icon.alpha_composite(place(art, big, shape * 0.40, big / 2))
    # Adaptive foreground: 108 dp canvas, the dog inside the 66 dp circle.
    fbig = 4 * 432
    foreground = place(art, fbig, fbig * (SAFE_DP / 2) / ADAPTIVE_DP, fbig / 2)

    sizes = []
    for name, k in DENSITIES.items():
        px = round(LEGACY_DP * k)
        fpx = round(ADAPTIVE_DP * k)
        d = f"mipmap-{name}"
        outs = (
            premul_resize(legacy, (px, px)),
            premul_resize(round_icon, (px, px)),
            premul_resize(foreground, (fpx, fpx)),
        )
        write_png(outs[0], "android", "res", d, "ic_launcher.png")
        write_png(outs[1], "android", "res", d, "ic_launcher_round.png")
        write_png(outs[2], "android", "res", d, "ic_launcher_foreground.png")
        sizes.append((name, px, fpx, outs))
    write_text(ADAPTIVE_XML, "android", "res", "mipmap-anydpi-v26", "ic_launcher.xml")
    write_text(ADAPTIVE_XML, "android", "res", "mipmap-anydpi-v26", "ic_launcher_round.xml")
    write_text(COLOR_XML.format(hex=hexbg), "android", "res", "values", "ic_launcher_background.xml")

    preview(master, art.crop((x0, y0, x1, y1)), foreground, bg, sizes)


def adaptive_preview(foreground, bg, size, shape):
    """The adaptive icon as a launcher would show it: 72 of 108 dp visible,
    masked to a circle or a rounded square."""
    full = Image.new("RGBA", foreground.size, bg + (255,))
    full.alpha_composite(foreground)
    n = foreground.width
    cut = round(n * (18 / 108))
    vis = full.crop((cut, cut, n - cut, n - cut))
    vis = premul_resize(vis, (size, size))
    vis.putalpha(circle_mask(size) if shape == "circle" else rounded_mask(size, corner=0.30))
    return vis


def preview(master, dog, foreground, bg, sizes):
    """Contact sheet: the same panel on a light and on a dark backdrop. Each
    panel: the 1024 master and the dog alone (both shown at 512), the
    adaptive icon under a circle and a rounded-square mask; every density's
    legacy square and round icon at actual size; every density's adaptive
    foreground at actual size, on the background colour."""
    pad, cap = 24, 16
    W = 1420
    panel_h = (cap + 512 + pad) + (cap + 192 + pad) + (cap + 432 + pad)
    sheet = Image.new("RGBA", (W, 2 * panel_h), (236, 236, 240, 255))
    sheet.alpha_composite(Image.new("RGBA", (W, panel_h), (40, 40, 48, 255)), (0, panel_h))
    draw = ImageDraw.Draw(sheet)

    for top, fill in ((0, (20, 20, 20)), (panel_h, (230, 230, 230))):

        def put(img, x, y, text, fill=fill):
            sheet.alpha_composite(img, (x, y + cap))
            draw.text((x, y + 2), text, fill=fill)

        y = top + pad // 2
        x = pad
        put(premul_resize(master, (512, 512)), x, y, "icon-1024 (shown at 512)")
        x += 512 + pad
        fg = Image.new("RGBA", (512, 512), (0, 0, 0, 0))
        dd = premul_resize(dog, (512, round(512 * dog.height / dog.width)))
        fg.alpha_composite(dd, (0, (512 - dd.height) // 2))
        put(fg, x, y, "foreground-1024 (the dog alone, shown at 512)")
        x += 512 + pad
        put(adaptive_preview(foreground, bg, 192, "circle"), x, y, "adaptive, circle mask")
        put(
            adaptive_preview(foreground, bg, 192, "rounded"),
            x,
            y + 192 + cap + pad,
            "adaptive, rounded mask",
        )

        y += cap + 512 + pad
        x = pad
        for name, px, _fpx, outs in sizes:
            base = y + (192 - px)
            put(outs[0], x, base, f"{name} {px}")
            put(outs[1], x + px + 8, base, "round")
            x += 2 * px + 8 + pad

        y += cap + 192 + pad
        x = pad
        for name, _px, fpx, outs in sizes:
            fgv = Image.new("RGBA", outs[2].size, bg + (255,))
            fgv.alpha_composite(outs[2])
            put(fgv, x, y + (432 - fpx), f"foreground {name} {fpx}")
            x += fpx + pad
    write_png(sheet.convert("RGB"), "preview.png")


if __name__ == "__main__":
    main()
