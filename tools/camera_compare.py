"""Compares camera logs per zoom step: the running 1.2 client's (tools/camera_probe.py) against aa-client's (AA_CAMERA_LOG), the
same columns. A step is a stretch where the wheel's distance holds and the camera has settled on it (distance = wanted) for at
least --hold seconds with the feet known; each step's numbers are medians over it.

    py -3 tools/camera_compare.py <native.csv> [<ours.csv> ...]        a table per file, then each of ours against the native one
    py -3 tools/camera_compare.py <file.csv> --steps                    only that file's steps

Columns of the table: wanted (m), samples, cam (the camera's height above the feet), target (the followed point's), look (where the
view's centre passes over the feet), pitch (degrees), distance (m). The comparison matches each native step to the step of ours
with the nearest wanted distance (within 0.05 m) and prints the differences.
"""
import argparse
import csv
import gzip
import statistics
import sys


def rows(path):
    # a `.csv.gz` (the committed native run) reads the same as a plain `.csv`
    with (gzip.open(path, 'rt', encoding='utf-8') if path.endswith('.gz') else open(path, encoding='utf-8')) as f:
        lines = [l for l in f if not l.startswith('#')]
    for r in csv.DictReader(lines):
        yield r


def num(r, k):
    v = r.get(k, '')
    try:
        return float(v)
    except ValueError:
        return None


def steps(path, hold=0.5, pitch_tol=1.0):
    """The settled stretches: (wanted, [rows]). A stretch breaks when the wanted distance, or the pitch by more than pitch_tol
    degrees, changes."""
    out, cur, key = [], [], None
    for r in rows(path):
        if r.get('mode', '1') != '1':
            continue
        w, d, p, fz = num(r, 'wanted'), num(r, 'distance'), num(r, 'pitch_deg'), num(r, 'feet_z')
        if None in (w, d, p, fz) or abs(d - w) > 0.01:
            if cur:
                out.append((key, cur))
            cur, key = [], None
            continue
        if key is not None and (abs(w - key[0]) > 1e-3 or abs(p - key[1]) > pitch_tol):
            out.append((key, cur))
            cur = []
        if not cur:
            key = (w, p)
        cur.append(r)
    if cur:
        out.append((key, cur))
    keep = []
    for (w, _), rs in out:
        t0, t1 = num(rs[0], 'time'), num(rs[-1], 'time')
        if t1 - t0 >= hold:
            keep.append((w, rs))
    return keep


def summary(rs):
    med = lambda k: statistics.median([v for v in (num(r, k) for r in rs) if v is not None] or [float('nan')])
    return {k: med(c) for k, c in (('cam', 'cam_above_feet'), ('target', 'target_above_feet'), ('look', 'look_above_feet'),
                                   ('pitch', 'pitch_deg'), ('distance', 'distance'), ('fov', 'fov_deg'), ('zoffset', 'zoffset'))}


def table(path, hold):
    st = [(w, len(rs), summary(rs)) for w, rs in steps(path, hold)]
    print(f'\n{path}')
    print(f'  {"wanted":>7} {"n":>5} {"cam":>7} {"target":>7} {"look":>7} {"pitch":>7} {"dist":>7} {"zoff":>6} {"fov":>6}')
    for w, n, s in st:
        print(f'  {w:7.3f} {n:5d} {s["cam"]:7.3f} {s["target"]:7.3f} {s["look"]:7.3f} {s["pitch"]:7.2f} {s["distance"]:7.3f} '
              f'{s["zoffset"]:6.3f} {s["fov"]:6.2f}')
    return st


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('files', nargs='+')
    ap.add_argument('--hold', type=float, default=0.5)
    ap.add_argument('--steps', action='store_true')
    args = ap.parse_args()
    tables = [table(p, args.hold) for p in args.files]
    if args.steps or len(tables) < 2:
        return 0
    native = tables[0]
    for path, ours in zip(args.files[1:], tables[1:]):
        print(f'\n{path} - native, per native step (m, degrees)')
        print(f'  {"wanted":>7} {"cam":>7} {"target":>7} {"look":>7} {"pitch":>7}')
        for w, _, s in native:
            match = min(ours, key=lambda o: abs(o[0] - w), default=None)
            if not match or abs(match[0] - w) > 0.05:
                print(f'  {w:7.3f}  (no step of ours)')
                continue
            o = match[2]
            print(f'  {w:7.3f} {o["cam"] - s["cam"]:+7.3f} {o["target"] - s["target"]:+7.3f} {o["look"] - s["look"]:+7.3f} '
                  f'{o["pitch"] - s["pitch"]:+7.2f}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
