"""Compare world-look stills of the original 1.2 client with aa-client's (same camera, same hour).

    py -3 -I tools/worldlook/compare.py fit ORIG OURS [ORIG OURS ...]
        colour transform from ours to the original over all pairs, in linear light on 24 px blocks: luminance ratio, per-channel
        gain and hue (RGB per luminance) of the top (sky, canopy: the upper 30 %) and the bottom (ground: below 45 %)
    py -3 -I tools/worldlook/compare.py shadow LIT DIM [--from-y 600]
        how much light shade keeps: the per-pixel ratio DIM / LIT of two frames of the same view, e.g. the original with
        `e_shadows 0` (LIT) and 1 (DIM), or aa-client with `AA_TUNE=adapt=0` and `adapt=0,sun_gain=0` (the ambient alone)
    py -3 -I tools/worldlook/compare.py sheet OUT.png COLUMNS IMAGE...
        a grid of half size images with their file names

aa-client's window can be smaller than the original's 1920 x 1080 (the task bar): its stills are scaled to 1080 rows and cut to
the centre 1920 columns first (the field of view is vertical, so this is the original's frame).
"""
from __future__ import annotations

import argparse
import os
import sys

import numpy as np
from PIL import Image, ImageDraw

LUMA = np.array([0.2126, 0.7152, 0.0722])


def aligned(path: str, w: int = 1920, h: int = 1080) -> Image.Image:
    im = Image.open(path).convert("RGB")
    if im.size == (w, h):
        return im
    nw = round(im.size[0] * h / im.size[1])
    im = im.resize((nw, h), Image.LANCZOS)
    x0 = (nw - w) // 2
    return im.crop((x0, 0, x0 + w, h))


def linear(im: Image.Image) -> np.ndarray:
    a = np.asarray(im).astype(float) / 255.0
    return np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)


def fit(paths: list[str], block: int = 24) -> None:
    if len(paths) % 2:
        raise SystemExit("fit takes pairs: ORIG OURS ...")
    orig, ours, rows = [], [], []
    for a, b in zip(paths[0::2], paths[1::2]):
        s, o = aligned(a), aligned(b)
        size = (s.size[0] // block, s.size[1] // block)
        ls, lo = linear(s.resize(size, Image.BOX)), linear(o.resize(size, Image.BOX))
        orig.append(ls.reshape(-1, 3)); ours.append(lo.reshape(-1, 3))
        rows.append(np.repeat(np.arange(size[1])[:, None] / size[1], size[0], 1).reshape(-1))
    S, B, Y = np.concatenate(orig), np.concatenate(ours), np.concatenate(rows)
    for name, mask in (("top", Y < 0.3), ("bottom", Y >= 0.45), ("all", Y >= 0)):
        s, b = S[mask], B[mask]
        ls, lb = s @ LUMA, b @ LUMA
        ok = (ls > 0.01) & (lb > 0.01)
        s, b, ls, lb = s[ok], b[ok], ls[ok], lb[ok]
        gain = np.median(s / np.maximum(b, 1e-4), 0)
        hue_s, hue_b = np.median(s / ls[:, None], 0), np.median(b / lb[:, None], 0)
        print(f"{name:6s} blocks {ok.sum():5d}  luminance original/ours {np.median(ls / lb):.2f}  gain RGB {gain.round(2)}"
              f"  hue original {hue_s.round(2)} ours {hue_b.round(2)}")


def shadow(lit: str, dim: str, from_y: int) -> None:
    a, b = linear(aligned(lit))[from_y:] @ LUMA, linear(aligned(dim))[from_y:] @ LUMA
    ok = a > 0.02
    r = b[ok] / a[ok]
    shade = r < 0.6
    print("percentiles 5/10/25/50/75", np.percentile(r, [5, 10, 25, 50, 75]).round(3))
    print(f"shade (ratio < 0.6): {shade.mean() * 100:.1f} % of the pixels, median {np.median(r[shade]):.3f}" if shade.any() else "no shade")
    print("histogram 0..1.2 in 0.06 steps:", np.histogram(r, bins=20, range=(0, 1.2))[0].tolist())


def sheet(out: str, cols: int, files: list[str]) -> None:
    w, h = 960, 540
    rows = (len(files) + cols - 1) // cols
    im = Image.new("RGB", (w * cols, h * rows))
    draw = ImageDraw.Draw(im)
    for i, f in enumerate(files):
        x, y = (i % cols) * w, (i // cols) * h
        im.paste(aligned(f).resize((w, h)), (x, y))
        draw.rectangle([x, y, x + 260, y + 22], fill=(0, 0, 0))
        draw.text((x + 4, y + 4), os.path.basename(f), fill=(255, 255, 0))
    im.save(out)
    print(out)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("fit"); p.add_argument("images", nargs="+")
    p = sub.add_parser("shadow"); p.add_argument("lit"); p.add_argument("dim"); p.add_argument("--from-y", type=int, default=600)
    p = sub.add_parser("sheet"); p.add_argument("out"); p.add_argument("columns", type=int); p.add_argument("images", nargs="+")
    a = ap.parse_args(argv)
    if a.cmd == "fit":
        fit(a.images)
    elif a.cmd == "shadow":
        shadow(a.lit, a.dim, a.from_y)
    else:
        sheet(a.out, a.columns, a.images)
    return 0


if __name__ == "__main__":
    sys.exit(main())
