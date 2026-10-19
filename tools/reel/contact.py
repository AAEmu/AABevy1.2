"""Contact sheet of recorded frames: python contact.py FRAMES_DIR OUT.png START STOP STEP [COLS] (frame numbers, 1 based)."""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageOps

d, out, a, b, step = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
cols = int(sys.argv[6]) if len(sys.argv) > 6 else 4
picks = list(range(a, b + 1, step))
tw, th = 480, 270
rows = (len(picks) + cols - 1) // cols
img = Image.new("RGB", (tw * cols, th * rows), "black")
for i, k in enumerate(picks):
    f = d / f"f_{k:06d}.png"
    if not f.exists():
        continue
    fr = ImageOps.fit(Image.open(f).convert("RGB"), (tw, th), Image.LANCZOS)
    ImageDraw.Draw(fr).text((6, 6), f"frame {k} ({k / 30:.2f} s)", fill="yellow")
    img.paste(fr, ((i % cols) * tw, (i // cols) * th))
img.save(out)
