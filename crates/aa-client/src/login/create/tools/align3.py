import sys
import numpy as np
from PIL import Image, ImageFilter
# usage: align3.py step x0 y0 x1 y1 [models...]
# best (scale about the screen centre, dx, dy) of MY image so that it matches the reference, in full-res px
step = sys.argv[1]; x0, y0, x1, y1 = map(int, sys.argv[2:6])
want = sys.argv[6:]
models = [(r, g) for r in ['nuian', 'elf', 'hariharan', 'ferre'] for g in ['male', 'female']]
F = 4
def gradmap(path):
    im = Image.open(path).convert("L").filter(ImageFilter.GaussianBlur(4)).resize((1920//F, 1080//F), Image.BILINEAR)
    a = np.asarray(im, dtype=np.float32)
    gx = np.zeros_like(a); gy = np.zeros_like(a)
    gx[:, :-1] = np.abs(np.diff(a, axis=1)); gy[:-1, :] = np.abs(np.diff(a, axis=0))
    return gx + gy
def scaled(g, s):
    H, W = g.shape
    im = Image.fromarray(g)
    m = im.resize((max(1, int(W*s)), max(1, int(H*s))), Image.BILINEAR)
    out = np.zeros_like(g)
    cx, cy = W/2, H/2
    ox, oy = int(round(cx - cx*s)), int(round(cy - cy*s))
    ma = np.asarray(m)
    # paste with clipping
    sx0, sy0 = max(0, -ox), max(0, -oy)
    dx0, dy0 = max(0, ox), max(0, oy)
    w = min(ma.shape[1]-sx0, W-dx0); h = min(ma.shape[0]-sy0, H-dy0)
    out[dy0:dy0+h, dx0:dx0+w] = ma[sy0:sy0+h, sx0:sx0+w]
    return out
def xcorr(a, b):
    # circular cross-correlation of a (ref region, zero padded) with b: returns map over shifts
    fa = np.fft.rfft2(a); fb = np.fft.rfft2(b)
    return np.fft.irfft2(fa * np.conj(fb), s=a.shape)
rx0, ry0, rx1, ry1 = x0//F, y0//F, x1//F, y1//F
for r, g in models:
    if want and f"{r}_{g}" not in want: continue
    A = gradmap(f"./reference/character_create/step{step}_{r}_{g}/real.png")
    B = gradmap(f"./shots/create/cmp/step{step}_{r}_{g}.png")
    mask = np.zeros_like(A); mask[ry0:ry1, rx0:rx1] = 1
    Am = (A - A[ry0:ry1, rx0:rx1].mean()) * mask
    best = (-9, 1, 0, 0)
    for s in [0.7 + 0.0125*i for i in range(0, 51)]:
        Bs = scaled(B, s)
        Bm = (Bs - Bs[ry0:ry1, rx0:rx1].mean())
        c = xcorr(Am, Bm)   # c[dy,dx] = sum A(y,x) B(y-dy, x-dx): B shifted by (dx,dy) matches
        # limit shifts to +-40 px (full-res 160)
        H, W = A.shape
        for dy in list(range(0, 41)) + list(range(-40, 0)):
            row = c[dy % H]
            for dx in list(range(0, 41)) + list(range(-40, 0)):
                v = row[dx % W]
                if v > best[0]: best = (v, s, dx*F, dy*F)
    # true NCC at the best
    s, dx, dy = best[1], best[2]//F, best[3]//F
    Bs = scaled(B, s); Bsh = np.roll(np.roll(Bs, dy, axis=0), dx, axis=1)
    a = A[ry0:ry1, rx0:rx1]; b = Bsh[ry0:ry1, rx0:rx1]
    a = a - a.mean(); b = b - b.mean()
    ncc = float((a*b).sum() / (np.sqrt((a*a).sum()*(b*b).sum()) + 1e-6))
    b0 = B[ry0:ry1, rx0:rx1]; b0 = b0 - b0.mean()
    ncc0 = float((a*b0).sum() / (np.sqrt((a*a).sum()*(b0*b0).sum()) + 1e-6))
    print(f"{r:10s} {g:7s} ncc {ncc0:.3f} -> {ncc:.3f}  scale {best[1]:.3f} shift dx={best[2]:+d} dy={best[3]:+d}")
