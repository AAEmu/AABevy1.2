"""Contact sheets of the skill audit frames: one PNG per skill from `<dir>/<id>_<n>.png`.

usage: python tools/skill_sheets.py FRAMES_DIR OUT_DIR [--tsv data/extract/aa-client-combat/skill_coverage.tsv] [--keep]
The frames are 960x540 (`--size 960x540`); each skill's frames are scaled to 480x270 and put in a row under a label with the
skill's id, name and clips. The frames are deleted after the sheet is made unless --keep is given.
"""
import csv
import glob
import os
import re
import sys
from PIL import Image, ImageDraw

frames, out = sys.argv[1], sys.argv[2]
tsv = 'data/extract/aa-client-combat/skill_coverage.tsv'
if '--tsv' in sys.argv:
    tsv = sys.argv[sys.argv.index('--tsv') + 1]
keep = '--keep' in sys.argv
info = {}
if os.path.exists(tsv):
    for r in csv.DictReader(open(tsv, encoding='utf-8'), delimiter='\t'):
        info[r['id']] = r
os.makedirs(out, exist_ok=True)
ids = sorted({re.match(r'(b?\d+)_', os.path.basename(f)).group(1) for f in glob.glob(os.path.join(frames, '*_*.png'))}, key=lambda x: (x.startswith('b'), int(x.lstrip('b'))))
for sid in ids:
    files = sorted(glob.glob(os.path.join(frames, f'{sid}_*.png')), key=lambda f: int(re.search(r'_(\d+)\.png$', f).group(1)))
    imgs = [Image.open(f).convert('RGB').resize((480, 270), Image.LANCZOS) for f in files]
    cols = min(len(imgs), 4)
    rows = (len(imgs) + cols - 1) // cols
    sheet = Image.new('RGB', (480 * cols, 270 * rows + 36), (20, 20, 20))
    for i, im in enumerate(imgs):
        sheet.paste(im, ((i % cols) * 480, 36 + (i // cols) * 270))
    r = info.get(sid, {})
    label = f"{sid} {r.get('name', '')}  [{r.get('set', '')}]  {r.get('clips', '')}"[:240]
    ImageDraw.Draw(sheet).text((6, 4), label, fill=(255, 255, 255))
    ImageDraw.Draw(sheet).text((6, 20), f"cast {r.get('cast_ms', '')} ms  chan {r.get('chan_ms', '')} ms  plot {r.get('plot', '')}  fx {r.get('fx_class', '')}", fill=(200, 200, 120))
    sheet.save(os.path.join(out, f'{sid}.png'), optimize=True)
    if not keep:
        for f in files:
            os.remove(f)
print(len(ids), 'sheets in', out)
