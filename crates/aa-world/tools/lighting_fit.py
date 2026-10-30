"""Renders the real-client reference views with the `fly` example and compares named regions (mean sRGB, linear luminance).

    python -I crates/aa-world/tools/lighting_fit.py render [--tag base] [--tune "ambient_dome=0"] [--views spawn,palace]
    python -I crates/aa-world/tools/lighting_fit.py compare [--tag base] [--views ...]
    python -I crates/aa-world/tools/lighting_fit.py sheet [--tag base]          one image: real | ours per view

Needs `cargo build -p aa-world --example fly` and AA_PAK. Renders go to shots/aa-world/lighting_fit/<tag>/<view>.png (git-ignored);
the real pictures are the local-only `reference/ingame/*/real*.png`. Each view is the capture's own camera (`camera*.json`), its
client-local origin shift and its game hour, with the camera as the time of day area subject (`--subject`, as the player). Regions are
boxes in 1920 x 1080 pixels chosen clear of the HUD and the player character; the score is the mean absolute difference of the
regions' sRGB means, and the log2 ratio of their linear luminance (stops; positive = ours brighter).
"""
import argparse, json, os, subprocess, sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..', '..'))
REF = os.path.join(ROOT, 'reference', 'ingame')
OUT = os.path.join(ROOT, 'shots', 'aa-world', 'lighting_fit')
FLY = os.path.join(ROOT, 'target', 'debug', 'examples', 'fly.exe' if os.name == 'nt' else 'fly')

# name: (world, reference dir, picture, camera json, shift, regions {name: (x0, y0, x1, y1)})
VIEWS = {
    'spawn': ('main_world', 'spawn', 'real.png', 'camera.json', '11264,11264,0', {
        'sky': (700, 60, 1300, 200), 'trees': (60, 330, 420, 520), 'grass': (1250, 700, 1650, 820),
        'rock': (1450, 450, 1700, 560)}),
    'palace': ('main_world', 'royal_palace', 'real.png', 'camera.json', '9216,6144,0', {
        'sky': (700, 40, 1250, 160), 'wall': (60, 250, 400, 500), 'cobbles': (1250, 760, 1650, 900)}),
    'palace90': ('main_world', 'royal_palace', 'real_turned90.png', 'camera_turned90.json', '9216,6144,0', {
        'sky': (700, 40, 1250, 160), 'roof': (60, 300, 400, 450), 'cobbles': (1250, 760, 1650, 900)}),
    'palace90w': ('main_world', 'royal_palace', 'real_turned90_wide.png', 'camera_turned90_wide.json', '9216,6144,0', {
        'sky': (700, 40, 1250, 160), 'ground': (1250, 760, 1650, 900)}),
    'mirage': ('arche_mall_world', 'mirage_isle', 'real_noui.png', 'camera_noui.json', '3072,3072,0', {
        'sky': (1480, 80, 1580, 260), 'wall': (1100, 120, 1260, 450), 'floor': (0, 780, 800, 1080),
        'pillar': (1740, 120, 1880, 520)}),
    'mirage1': ('arche_mall_world', 'mirage_isle', 'real_turn1.png', 'camera_turn1.json', '3072,3072,0', {
        'wall': (300, 100, 820, 600), 'floor': (0, 780, 800, 1080)}),
    'mirage2': ('arche_mall_world', 'mirage_isle', 'real_turn2.png', 'camera_turn2.json', '3072,3072,0', {
        'sky_high': (0, 0, 700, 120), 'sky_low': (0, 330, 300, 430), 'pillar': (480, 100, 620, 600),
        'floor': (0, 780, 800, 1080)}),
    'mirage3': ('arche_mall_world', 'mirage_isle', 'real_turn3.png', 'camera_turn3.json', '3072,3072,0', {
        'sky_high': (0, 0, 700, 200), 'sky_low': (0, 500, 600, 560), 'sea': (0, 600, 300, 680),
        'pillar': (1380, 150, 1550, 500), 'floor': (0, 780, 800, 1080)}),
}


def render(tag, tune, views, after):
    os.makedirs(os.path.join(OUT, tag), exist_ok=True)
    env = dict(os.environ)
    if tune:
        env['AA_TUNE'] = tune
    for name in views:
        world, d, _, cam, shift, _ = VIEWS[name]
        cam_path = os.path.join(REF, d, cam)
        hour = json.load(open(cam_path, encoding='utf-8'))['cvars']['e_time_of_day']
        shot = os.path.join(OUT, tag, name + '.png')
        args = [FLY, '--world', world, '--camera-json', cam_path, '--shift', shift, '--hour', str(hour), '--subject',
                '--shot', shot, '--shot-after', str(after)]
        r = subprocess.run(args, env=env, capture_output=True, text=True, errors='replace')
        fps = [l.split('fps=')[1].split()[0] for l in r.stdout.splitlines() + r.stderr.splitlines() if 'fps=' in l]
        print(name, 'ok' if os.path.exists(shot) else 'FAILED', 'fps', fps[-1] if fps else '?', flush=True)


def stats(img, box):
    import numpy as np
    a = np.asarray(img.crop(box).convert('RGB')).astype(float) / 255.0
    lin = np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)
    return a.reshape(-1, 3).mean(0) * 255.0, float((lin @ [0.2126, 0.7152, 0.0722]).mean())


def compare(tag, views):
    import math
    from PIL import Image
    total, n = 0.0, 0
    for name in views:
        _, d, pic, _, _, regions = VIEWS[name]
        real = Image.open(os.path.join(REF, d, pic))
        ours = Image.open(os.path.join(OUT, tag, name + '.png'))
        for region, box in regions.items():
            (rc, rl), (oc, ol) = stats(real, box), stats(ours, box)
            err = float(abs(rc - oc).mean())
            total += err; n += 1
            print('%-10s %-9s real %3d %3d %3d  ours %3d %3d %3d  diff %5.1f  stops %+5.2f' % (
                name, region, *rc.round(), *oc.round(), err, math.log2(max(ol, 1e-5) / max(rl, 1e-5))))
    print('%s: mean abs sRGB difference %.2f over %d regions' % (tag, total / max(n, 1), n))


def sheet(tag, views):
    from PIL import Image
    w, h = 640, 360
    out = Image.new('RGB', (w * 2, h * len(views)))
    for i, name in enumerate(views):
        _, d, pic, _, _, _ = VIEWS[name]
        out.paste(Image.open(os.path.join(REF, d, pic)).convert('RGB').resize((w, h)), (0, i * h))
        out.paste(Image.open(os.path.join(OUT, tag, name + '.png')).convert('RGB').resize((w, h)), (w, i * h))
    path = os.path.join(OUT, tag + '_sheet.png')
    out.save(path)
    print(path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('cmd', choices=['render', 'compare', 'sheet'])
    ap.add_argument('--tag', default='base')
    ap.add_argument('--tune', default='')
    ap.add_argument('--views', default=','.join(VIEWS))
    ap.add_argument('--after', type=float, default=15)
    a = ap.parse_args()
    views = a.views.split(',')
    if a.cmd == 'render':
        render(a.tag, a.tune, views, a.after)
        compare(a.tag, views)
    elif a.cmd == 'compare':
        compare(a.tag, views)
    else:
        sheet(a.tag, views)


if __name__ == '__main__':
    sys.exit(main())
