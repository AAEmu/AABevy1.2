"""Rebuild the art-only preview from manifest.json. Requires Python 3 + Pillow."""
from pathlib import Path
import json
import math
import random
from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parent
M = json.loads((ROOT / "manifest.json").read_text())
S = M["preview"]["size_px"][0] / M["canvas_px"][0]
SIZE = tuple(M["preview"]["size_px"])
IMAGES = {f: Image.open(ROOT / f).convert("RGBA") for f in
          [l["file"] for l in M["layers"]] + [M["panel"]["file"], M["motes"]["file"]]}
RNG = random.Random(M["motes"]["seed"])
P = M["motes"]
X0, Y0, X1, Y1 = P["spawn_area_px"]
MOTES = [dict(x=RNG.uniform(X0,X1), y=RNG.uniform(Y0,Y1),
              speed=RNG.uniform(*P["rise_speed_px_s"]),
              sway=RNG.uniform(*P["sway_amplitude_px"]),
              period=RNG.uniform(*P["sway_period_s"]),
              life=RNG.uniform(*P["lifetime_s"]),
              size=RNG.uniform(*P["size_px"]), phase=RNG.random()*math.tau,
              age=RNG.random()*32) for _ in range(P["count"])]

def over(dst, src, center, scale=1, alpha=1):
    sz = tuple(max(1,round(n*scale*S)) for n in src.size)
    src = src.resize(sz,Image.Resampling.LANCZOS)
    if alpha < 1:
        src.putalpha(src.getchannel("A").point(lambda a: round(a*max(0,alpha))))
    dst.alpha_composite(src, (round(center[0]*S-sz[0]/2), round(center[1]*S-sz[1]/2)))

def frame(t):
    dst=Image.new("RGBA",SIZE,(0,0,0,255))
    # Gentle representative mouse excursion, panel remains fixed.
    mouse=(220*math.sin(t*0.5), 100*math.sin(t*0.3))
    for l in sorted(M["layers"],key=lambda a:a["z"]):
        b=l["breathing"]
        wave=math.sin(math.tau*t/b["period_s"]+b["phase_rad"])
        scale=l["scale"]*(1+b["scale_amplitude"]*wave)
        alpha=max(0,min(1,l["alpha"]+b["alpha_amplitude"]*wave))
        x,y=[l["position_px"][i]+l["drift_px_s"][i]*t+mouse[i]*l["parallax"] for i in (0,1)]
        src=IMAGES[l["file"]]
        if l["tile_x"]:
            width=src.width*scale
            x=(x+width/2)%width-width/2
            for j in range(-1,4):
                over(dst,src,(x+j*width,y),scale,alpha)
        else:
            over(dst,src,(x,y),scale,alpha)
    sprite=IMAGES[P["file"]].copy()
    tint=Image.new("RGBA",sprite.size,tuple(round(v*255) for v in P["tint_rgba"]))
    sprite=ImageChops.multiply(sprite,tint)
    for p in MOTES:
        age=(t+p["age"])%p["life"]
        fade=min(1,age/P["fade_in_s"],(p["life"]-age)/P["fade_out_s"])
        x=p["x"]+p["sway"]*math.sin(math.tau*t/p["period"]+p["phase"])
        over(dst,sprite,(x,p["y"]-p["speed"]*age),p["size"]/sprite.width,fade)
    panel=M["panel"]
    center=[M["canvas_px"][i]/2+panel["offset_px"][i] for i in (0,1)]
    over(dst,IMAGES[panel["file"]],center,1,panel["alpha"])
    return dst.convert("RGB")

if __name__=="__main__":
    fps=M["preview"]["fps"]
    duration=M["preview"]["duration_s"]
    first=frame(0)
    frames=[]
    for i in range(round(fps*duration)):
        t=i/fps
        im=frame(t)
        crossfade=M["preview"]["loop_crossfade_s"]
        if t>duration-crossfade:
            im=Image.blend(im,first,(t-duration+crossfade)/(crossfade-1/fps))
        frames.append(im)
    # A shared palette prevents frame-to-frame colour flicker.
    palette=first.quantize(colors=96,method=Image.Quantize.MEDIANCUT)
    frames=[im.quantize(palette=palette,dither=Image.Dither.NONE) for im in frames]
    frames[0].save(ROOT/"preview.gif",save_all=True,append_images=frames[1:],
                   duration=round(1000/fps),loop=0,optimize=True,disposal=1)
    first.save(ROOT/"preview_still.png",optimize=True)
    assert (ROOT/"preview.gif").stat().st_size<4_000_000
    print("Saved 6-second loop and still to",ROOT)
