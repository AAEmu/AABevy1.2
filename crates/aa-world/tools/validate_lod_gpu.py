"""Check paired red/green footprints: a hole is black; an additive double draw is yellow.
Requires Pillow. Pass PNG paths (1024x768); grid_before.png is the hard-switch control.
The 17 central 64x64 ROIs avoid quad edges and cover 256 complete Bayer cells each.
"""
from pathlib import Path
from PIL import Image
import json,sys

thresholds=[0x0a020800,0x060e040c,0x09010b03,0x050d070f]
for name in sys.argv[1:]:
    p=Path(name)
    im=Image.open(p).convert('RGB')
    assert im.size==(1024,768), im.size
    bad=holes=duplicates=0
    levels=[]
    for n in range(17):
        cx=512+((n%5)*2-4)*76.8
        cy=384-(3-(n//5)*2)*76.8
        red=green=0
        for y in range(round(cy)-32,round(cy)+32):
            for x in range(round(cx)-32,round(cx)+32):
                rgb=im.getpixel((x,y))
                red+=rgb==(255,0,0)
                green+=rgb==(0,255,0)
                holes+=rgb==(0,0,0)
                duplicates+=rgb==(255,255,0)
                step=(0 if n<8 else 16) if 'before' in p.stem else n
                threshold=(thresholds[y%4]>>((x%4)*8))&255
                expected=(255,0,0) if step+threshold<16 else (0,255,0)
                bad+=rgb!=expected
        levels.append([red,green])
    out=dict(path=str(p),checked=17*4096,mask_mismatches=bad,holes=holes,duplicates=duplicates,red_green_counts=levels)
    print(json.dumps(out))
    assert bad==holes==duplicates==0,out
