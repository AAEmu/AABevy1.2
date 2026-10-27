"""Contact sheet of the eight step 1 captures: real | render per figure (python -I sheet.py <render prefix> <out.png> [scale]).

`<render prefix>` is the path prefix of the renders, e.g. shots/materials/after_ for
shots/materials/after_step1_elf_female.png."""
import sys
from pathlib import Path
from PIL import Image

REF = Path(__file__).resolve().parents[3] / "reference"
prefix, out = sys.argv[1:3]
scale = float(sys.argv[3]) if len(sys.argv) > 3 else 0.5
names = ["nuian_male", "nuian_female", "elf_male", "elf_female", "hariharan_male", "hariharan_female", "ferre_male", "ferre_female"]
box = (740, 20, 1180, 1080)
w, h = int((box[2] - box[0]) * scale), int((box[3] - box[1]) * scale)
sheet = Image.new("RGB", (w * 2 * 4, h * 2))
for i, n in enumerate(names):
    real = Image.open(f"{REF}/character_create/step1_{n}/real.png").convert("RGB")
    mine = Image.open(f"{prefix}step1_{n}.png").convert("RGB").resize(real.size)
    x, y = (i % 4) * w * 2, (i // 4) * h
    sheet.paste(real.crop(box).resize((w, h)), (x, y))
    sheet.paste(mine.crop(box).resize((w, h)), (x + w, y))
sheet.save(out)
