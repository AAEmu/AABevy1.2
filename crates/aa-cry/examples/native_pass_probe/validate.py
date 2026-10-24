"""Pixel assertions on actual color/depth/normal/motion attachments and draw records.

These are SUPPLIED mechanism controls, not native scene or Illum alpha-law goldens.
Usage: python .../validate.py [output directory from run.py]
"""
import csv
import json
import re
from pathlib import Path
import struct
import sys
import zipfile
from PIL import Image

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "data/extract/aa-cry/native-pass-probe-20261024/verified"
checks = 0


def attachment_bytes(directory, name):
    if (directory / name).exists():
        return (directory / name).read_bytes()
    with zipfile.ZipFile(directory / "attachments.zip") as archive:
        return archive.read(name)


def attachment_names(directory):
    names = [p.name for p in directory.glob("*.bin")]
    if names:
        return names
    with zipfile.ZipFile(directory / "attachments.zip") as archive:
        return [name for name in archive.namelist() if name.endswith(".bin")]


def check(value, context):
    global checks
    checks += 1
    assert value, context


class Attachment:
    def __init__(self, directory, view, frame, stage, kind):
        self.path = directory / f"v{view}-f{frame}-{stage}-{kind}.bin"
        self.meta = dict(line.split("=", 1) for line in self.path.with_suffix(".txt").read_text().splitlines())
        self.width = int(self.meta["width"])
        self.height = int(self.meta["height"])
        self.pitch = int(self.meta["row_pitch"])
        self.format = self.meta["format"]
        self.data = attachment_bytes(directory, self.path.name)
        self.pixel_size = 8 if self.format == "Rgba16Float" else 4

    def sample(self, view, x, y):
        px = round(view * self.width / 2 + (x + 4) / 8 * self.width / 2)
        py = round((6 - y) / 12 * self.height)
        offset = py * self.pitch + px * self.pixel_size
        formats = {"Depth32Float": "<f", "Rg16Float": "<ee", "Rgb10a2Unorm": "<I", "Rgba16Float": "<eeee"}
        if self.format in formats:
            return struct.unpack_from(formats[self.format], self.data, offset)
        rgba = struct.unpack_from("4B", self.data, offset)
        return (rgba[2], rgba[1], rgba[0], rgba[3]) if self.format.startswith("Bgra") else rgba


def green(rgba):
    return rgba[1] > 0.9 and rgba[0] == 0 and rgba[2] == 0


def validate_case(name, report):
    directory = OUT / name
    mode = "all"
    command = report["command"]
    if "--attachments" in command:
        mode = command[command.index("--attachments") + 1]
    log = (directory / "run.log").read_text()
    check("actual DrawMesh missing-mesh => draw Ok, trailing recorder 0" in log, (name, "real Skip witness"))
    if "--no-z" not in command:
        check(any(int(v) > 0 for v in re.findall(r"indirect_commands=(\d+)", log)), (name, "actual indirect Z execution"))
    if mode in ("motion", "all"):
        check(f"actual_bindless={'false' if name == 'ordinary' else 'true'}" in log, (name, "actual material bindings"))
    for view in (0, 1):
        has_z = view == 0 and "--no-z" not in command
        for frame in (360, 361):
            state = {int(r["group"]): r for r in csv.DictReader((directory / f"v{view}-f{frame}-state.csv").open())}
            for group in range(6):
                submitted = has_z and group in (0, 1, 2, 5) and (group != 5 or frame % 2 == 0)
                check(state[group]["submitted_z"] == str(submitted).lower(), (name, view, frame, group, "submission"))
                check(int(state[group]["instance_range"]) == (3 if submitted else 0), (name, view, frame, group, "automatic instancing"))
                check(state[group]["paired_ready"] == str(group != 3).lower(), (name, view, frame, group, "pair readiness"))

            kinds = ["depth", "colour"] + (["normal"] if mode in ("all", "normal") else []) + (["motion"] if mode in ("all", "motion") else [])
            a = {kind: Attachment(directory, view, frame, "motion", kind) for kind in kinds}
            before = {kind: Attachment(directory, view, frame, "before-motion", kind) for kind in kinds}
            zdepth = Attachment(directory, view, frame, "z", "depth")
            for kind in ("depth", "normal", "colour"):
                if kind in a:
                    check(a[kind].data == before[kind].data, (name, view, frame, kind, "motion must preserve attachment"))
            if has_z:
                check(zdepth.data == before["depth"].data, (name, view, frame, "forward Equal/no depth writes"))
            if "motion" in a:
                check(set(struct.iter_unpack("<ee", before["motion"].data)) == {(0.125, -0.25)}, (name, view, frame, "all motion writers split before replay"))
            direction = 1 if frame % 2 == 0 else -1
            for group in range(6):
                for column in range(3):
                    for side in (0, 1):
                        x = -2.4 + column * 2.4 + (-0.325 if side == 0 else 0.325)
                        y = 4.8 - group * 1.9
                        active = group != 4 and (not has_z or group != 3) and (not has_z or group != 5 or frame % 2 == 0)
                        if has_z:
                            coverage = active and (group not in (0, 1) or group == 0 and side == 1)
                        else:
                            coverage = active and group != 2
                        colour = a["colour"].sample(view, x, y)
                        check(green(colour) == coverage, (name, view, frame, group, column, side, "forward", colour))
                        if has_z and group == 2:
                            check(colour[3] == 0, (name, view, frame, "post-Z output alpha preserved independently of coverage"))
                        if not coverage:
                            check(colour[:2] == (0, 0), (name, view, frame, group, "no spurious color"))
                        if group == 2 and not has_z:
                            check(colour[2] > 0, (name, view, frame, "visible stock backing"))
                        depth = a["depth"].sample(view, x, y)[0]
                        check((depth > 0) == (coverage or group == 2), (name, view, frame, group, "depth", depth))
                        if "normal" in a:
                            normal = a["normal"].sample(view, x, y)[0] & 0x3fffffff
                            check((normal != 0) == (has_z and coverage or not has_z and group == 2), (name, view, frame, group, "normal coverage (stock backing retains its own Z)"))
                        if "motion" in a:
                            motion = a["motion"].sample(view, x, y)
                            native_writes = active and group != 1 and (side == 1 or group == 3)
                            if not native_writes:
                                expected = (direction * 0.005, 0.0) if group == 2 and not has_z else (0.125, -0.25)
                            else:
                                expected = (direction * 0.0025, 0.0)
                            check(max(abs(motion[i] - expected[i]) for i in (0, 1)) < 0.000004, (name, view, frame, group, side, "velocity preservation", motion, expected))
            if "motion" in a:
                for center in (-3.4, 3.4):
                    for side, raw_alpha in enumerate((0.3, 0.8)):
                        motion = a["motion"].sample(view, center + (-0.2 if side == 0 else 0.2), -5.6)
                        expected = direction * 0.0025 * raw_alpha * 0.5
                        check(abs(motion[0] - expected) < 0.000002 and motion[1] == 0, (name, view, frame, "independent Blend/Add admission and explicit weight", motion, expected))


def main():
    captures = json.loads((OUT / "captures.json").read_text())
    for name, report in captures.items():
        if name in ("bindless", "ordinary", "depth", "normal", "motion", "noz-motion", "targets", "occlusion"):
            validate_case(name, report)
            print("PASS", name)
    if "bindless" in captures and "ordinary" in captures:
        for name in attachment_names(OUT / "bindless"):
            check(attachment_bytes(OUT / "bindless", name) == attachment_bytes(OUT / "ordinary", name), ("bindless/ordinary exact attachment parity", name))
    shadows = {}
    for prefix in ("shadow", "point"):
        if f"{prefix}-stock" not in captures:
            continue
        original = Image.open(OUT / f"{prefix}-stock/shot.png").convert("RGB")
        adapted = Image.open(OUT / f"{prefix}-split/shot.png").convert("RGB")
        disabled = Image.open(OUT / f"{prefix}-off/shot.png").convert("RGB")
        check(original.tobytes() == adapted.tobytes(), (prefix, "stock shadow/main paths preserved"))
        changed = sum(a != b for a, b in zip(original.getdata(), disabled.getdata()))
        check(changed > 100, (prefix, "actual visible receiver shadow discriminator", changed))
        shadows[prefix] = changed
        if prefix == "shadow" and "shadow-unmarked" in captures:
            unmarked = Image.open(OUT / "shadow-unmarked/shot.png").convert("RGB")
            check(original.tobytes() == unmarked.tobytes(), "unmarked cameras retain exact stock prepass dispatch")
    result = dict(assertions=checks, cases=list(captures), shadow_receiver_changed_pixels=shadows,
                  scope="Supplied Bevy renderer mechanism, NOT native material/input/GPU completion")
    (OUT / "validation.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
