"""Side by side of a compare shot and the real capture (python -I side_by_side.py <target> <shot.png> <out.png> [x0 y0 x1 y1]).

`target` is the folder under reference/ (character_create/step2_elf_female, character_select/1_character).
The optional box (in 1920x1080 pixels) crops both images to the same region; real left, shot right."""
import sys
from pathlib import Path
from PIL import Image

REF = Path(__file__).resolve().parents[3] / "reference"
target, shot, out = sys.argv[1:4]
real = Image.open(f"{REF}/{target}/real.png").convert("RGB")
mine = Image.open(shot).convert("RGB").resize(real.size)
if len(sys.argv) >= 8:
    box = tuple(int(v) for v in sys.argv[4:8])
    real, mine = real.crop(box), mine.crop(box)
sheet = Image.new("RGB", (real.width * 2, real.height))
sheet.paste(real, (0, 0))
sheet.paste(mine, (real.width, 0))
sheet.save(out)
