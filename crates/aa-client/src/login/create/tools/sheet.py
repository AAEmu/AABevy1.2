import sys
from PIL import Image, ImageDraw
# usage: sheet.py step out.png [cx0 cy0 cx1 cy1]  -> pairs ref|mine for the 8 models, optionally cropped
step = sys.argv[1]; out = sys.argv[2]
crop = tuple(map(int, sys.argv[3:7])) if len(sys.argv) >= 7 else (0, 0, 1920, 1080)
models = [(r, g) for r in ['nuian', 'elf', 'hariharan', 'ferre'] for g in ['male', 'female']]
cw, ch = crop[2]-crop[0], crop[3]-crop[1]
scale = min(480/cw, 1.0) if cw > 480 else 1.0
tw, th = int(cw*scale), int(ch*scale)
cols = 2 if tw <= 480 else 1
W = Image.new("RGB", (cols*2*tw+ (cols*2-1)*4, ((8+cols-1)//cols)*(th+4)), (255, 0, 255))
for i, (r, g) in enumerate(models):
    a = Image.open(f"./reference/character_create/step{step}_{r}_{g}/real.png").convert("RGB").crop(crop).resize((tw, th))
    try:
        b = Image.open(f"./shots/create/cmp/step{step}_{r}_{g}.png").convert("RGB").crop(crop).resize((tw, th))
    except Exception:
        b = Image.new("RGB", (tw, th), (0, 0, 0))
    x = (i % cols) * (2*tw+8); y = (i // cols) * (th+4)
    W.paste(a, (x, y)); W.paste(b, (x+tw+2, y))
    ImageDraw.Draw(W).text((x+4, y+4), f"{r} {g}", fill=(255, 255, 0))
W.save(out)
