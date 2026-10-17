import sys
import numpy as np
from PIL import Image
# usage: bbox.py step [region x0 y0 x1 y1]: bounding box of MY figure (diff against shots/create/bg)
step = sys.argv[1]
x0, y0, x1, y1 = (map(int, sys.argv[2:6])) if len(sys.argv) >= 6 else (0, 0, 1000, 1080)
models = [(r, g) for r in ['nuian', 'elf', 'hariharan', 'ferre'] for g in ['male', 'female']]
for r, g in models:
    a = np.asarray(Image.open(f"./shots/create/cmp/step{step}_{r}_{g}.png").convert("RGB"), dtype=np.int16)
    b = np.asarray(Image.open(f"./shots/create/bg/step{step}_{r}_{g}.png").convert("RGB"), dtype=np.int16)
    d = np.abs(a - b).sum(axis=2)
    m = d > 60
    m[:, :x0] = False; m[:, x1:] = False; m[:y0, :] = False; m[y1:, :] = False
    # drop specks: require the row / column to have several pixels
    rows = np.where(m.sum(axis=1) > 3)[0]; cols = np.where(m.sum(axis=0) > 3)[0]
    if len(rows) == 0:
        print(r, g, "none"); continue
    # torso centre: mean x of mask pixels in the middle third of the height
    t, bt = rows[0], rows[-1]
    mid = m[t + (bt - t)//3: t + 2*(bt - t)//3]
    cx = int(np.where(mid.any(axis=0))[0].mean())
    print(f"{r:10s} {g:7s} top {t} bottom {bt} height {bt-t} x {cols[0]}..{cols[-1]} mid-centre {cx}")
