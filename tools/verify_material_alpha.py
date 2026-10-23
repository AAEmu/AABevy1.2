"""Exercise the client example's real nested Illum/prepass rendering.

Build material_alpha_integration first, then pass that immutable executable.
Pillow is used only to inspect renderer screenshots.
"""
import argparse
import hashlib
import itertools
import json
import re
import subprocess
from pathlib import Path

from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("--output", type=Path, default=Path("shots/resume/alpha"))
    parser.add_argument("--lod-half", action="store_true", help="check active production LOD tags against the same-distance control")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    executable = args.executable.resolve()
    results = {}
    for ordinary, direct, low, view in itertools.product(
        (False, True), (False, True), (False, True), ("color", "depth", "normal")
    ):
        # The ordinary nested path is compared with both bindless paths.
        if ordinary and direct:
            continue
        if args.lod_half and low:
            continue
        key = f"{'direct' if direct else 'nested'}-{view}{'-low' if low else ''}{'-ordinary' if ordinary else ''}"
        shot = args.output / f"{key}.png"
        log = args.output / f"{key}.log"
        command = [str(executable), "--size", "256x256", "--view", view,
                   "--shot", str(shot), "--shot-after", "5"]
        command += ["--direct"] if direct else []
        command += ["--low-alpha"] if low else []
        command += ["--no-bindless"] if ordinary else []
        command += ["--lod-half"] if args.lod_half else []
        with log.open("w", encoding="utf-8") as output:
            run = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=90)
        text = log.read_text(encoding="utf-8")
        assert run.returncode == 0, (key, run.returncode, log)
        assert not re.search(r"\b(?:WARN|ERROR)\b|^\s*error:|panicked|module not found", text, re.I | re.M), log
        assert "nested alpha:" in text and shot.is_file(), (key, log)
        with Image.open(shot) as image:
            assert image.size == (256, 256), image.size
            rgb = image.convert("RGB")
            occupied = sum(any(pixel) for pixel in rgb.getdata())
            if low:
                assert occupied == 0, (key, occupied)
            else:
                minimum, maximum = (18000, 21000) if args.lod_half else (20000, 48000)
                assert minimum < occupied < maximum, (key, occupied)
            if view in ("depth", "normal"):
                # AlphaTest=.059 on the original material, X alpha 0..1.
                # Rectangle width=204.8px, centered at128: first surviving X=38.
                expected = None if low else (38, 26, 230, 230)
                assert rgb.getbbox() == expected, (key, rgb.getbbox(), expected)
            results[key] = {"occupied": occupied, "sha256": hashlib.sha256(shot.read_bytes()).hexdigest(),
                            "fps": [float(x) for x in re.findall(r"aa-dev: fps=([\d.]+)", text)]}
        print(f"PASS {key}: {occupied} covered pixels", flush=True)
    equalities = []
    for view, low in itertools.product(("color", "depth", "normal"), (False, True)):
        if args.lod_half and low:
            continue
        suffix = f"-{view}{'-low' if low else ''}"
        paths = [args.output / f"{prefix}{suffix}{ordinary}.png"
                 for prefix, ordinary in (("direct", ""), ("nested", ""), ("nested", "-ordinary"))]
        with Image.open(paths[0]) as reference:
            for path in paths[1:]:
                with Image.open(path) as other:
                    assert reference.convert("RGBA").tobytes() == other.convert("RGBA").tobytes(), path
                equalities.append([str(paths[0]), str(path)])
    report = {"executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(), "lod_half": args.lod_half,
              "captures": results, "exact_equalities": equalities}
    (args.output / "validation.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"PASS {len(results)} captures; {len(equalities)} exact image equalities", flush=True)


if __name__ == "__main__":
    main()
