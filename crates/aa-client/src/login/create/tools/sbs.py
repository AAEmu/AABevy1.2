import sys
from PIL import Image
# usage: sbs.py out.png ref.png mine.png x0 y0 x1 y1  -> side by side crop of both
out, a, b = sys.argv[1:4]
x0, y0, x1, y1 = map(int, sys.argv[4:8])
A = Image.open(a).convert("RGB").crop((x0, y0, x1, y1))
B = Image.open(b).convert("RGB").crop((x0, y0, x1, y1))
W = Image.new("RGB", (A.width + B.width + 6, A.height), (255, 0, 255))
W.paste(A, (0, 0)); W.paste(B, (A.width + 6, 0))
W.save(out)
