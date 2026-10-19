"""Stitches the row images of `equip_sweep --sheets` into labelled contact sheets (JPEG).

    python -I tools/equip_sweep/stitch.py shots/aa-anim/equip/_raw shots/aa-anim/equip [--keep]

Input : <raw>/<slot>_<model>_<tag>.tsv (cell list written by the example) and
        <raw>/<slot>_<model>_<tag>_<pose>_<view>_r<row>.png (one image per row of cells)
Output: <out>/<slot>_<model>_<tag>_<pose>_<view>.jpg, every cell labelled with the item id and level.
"""

import csv
import glob
import os
import re
import sys

from PIL import Image, ImageDraw, ImageFont


WIDTH = 2400  # contact sheets are scaled to this width (keeps the whole set under 300 MB)
BITMAP = ImageFont.load_default()  # PIL's truetype text crashes on some Windows builds


def text(sheet, xy, label, fill, scale=2, back=(0, 0, 0)):
    """Draws `label` with the bitmap font, enlarged `scale` times, on a solid background."""
    w, h = 6 * len(label) + 4, 13
    tile = Image.new("RGB", (w, h), back)
    ImageDraw.Draw(tile).text((2, 1), label, font=BITMAP, fill=fill)
    sheet.paste(tile.resize((w * scale, h * scale), Image.NEAREST), xy)


def main():
    raw, out = sys.argv[1], sys.argv[2]
    keep = "--keep" in sys.argv
    os.makedirs(out, exist_ok=True)
    for tsv in sorted(glob.glob(os.path.join(raw, "*.tsv"))):
        stem = os.path.basename(tsv)[:-4]
        with open(tsv, encoding="utf-8") as f:
            cells = list(csv.DictReader(f, delimiter="\t"))
        if not cells:
            continue
        cols = max(int(c["col"]) for c in cells) + 1
        rows = max(int(c["row"]) for c in cells) + 1
        shots = {}
        for png in glob.glob(os.path.join(raw, glob.escape(stem) + "_*_r*.png")):
            m = re.match(re.escape(stem) + r"_(.+)_(front|side|back)_r(\d+)\.png$", os.path.basename(png))
            if m:
                shots.setdefault((m.group(1), m.group(2)), {})[int(m.group(3))] = png
        for (pose, view), rowfiles in sorted(shots.items()):
            images = [Image.open(rowfiles[r]).convert("RGB") for r in range(rows) if r in rowfiles]
            if len(images) != rows:
                print("incomplete", stem, pose, view)
                continue
            w, h = images[0].size
            sheet = Image.new("RGB", (w, h * rows + 30), (18, 19, 23))
            for r, im in enumerate(images):
                sheet.paste(im, (0, 30 + r * h))
            text(sheet, (8, 2), f"{stem}  pose={pose}  view={view}", (255, 255, 255), back=(18, 19, 23))
            cw = w / cols
            for c in cells:
                x = int(int(c["col"]) * cw) + 4
                y = 30 + int(c["row"]) * h + 3
                status = "" if c["status"] == "ok" else " " + c["status"].split(":")[0]
                label = f'{c["item"]} L{c["level"]}{status}'
                text(sheet, (x, y), label, (255, 220, 90) if status else (255, 255, 255))
            dst = os.path.join(out, f"{stem}_{pose}_{view}.jpg")
            if sheet.width > WIDTH:
                sheet = sheet.resize((WIDTH, round(sheet.height * WIDTH / sheet.width)), Image.LANCZOS)
            sheet.save(dst, quality=72, optimize=True)
            if not keep:
                for p in rowfiles.values():
                    os.remove(p)
        if not keep:
            # the cell list stays next to the sheets
            os.replace(tsv, os.path.join(out, stem + ".tsv"))


if __name__ == "__main__":
    main()
