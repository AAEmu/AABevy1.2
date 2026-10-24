"""Pixel assertions on actual color/depth/normal/motion attachments and draw records.

These are SUPPLIED mechanism controls, not native scene or Illum alpha-law goldens.
Usage: python .../validate.py [output directory from run.py]
"""
import csv
import argparse
import hashlib
import json
import math
import re
from pathlib import Path
import struct
import sys
import zipfile
from PIL import Image

ROOT = Path(__file__).resolve().parents[4]
parser = argparse.ArgumentParser()
parser.add_argument("directory", nargs="?", type=Path, default=ROOT / "data/extract/aa-cry/native-pass-probe-review01-fix-20261024/verified")
parser.add_argument("--read-only", action="store_true", help="Validate without changing historical validation.json")
ARGS = parser.parse_args()
OUT = ARGS.directory
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
        check(f"actual_bindless={'false' if '--no-bindless' in command else 'true'}" in log, (name, "actual material bindings"))
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
                if name.startswith("readiness") and frame == 360:
                    # The deliberately deferred STOCK Z control still uses its
                    # ordinary main draw, which is allowed to provide its depth.
                    # Native post-Z depth remains unchanged everywhere else.
                    w, h = zdepth.width, zdepth.height
                    left = math.floor(view * w / 2 + (-1.6 + 4) / 8 * w / 2) - 1
                    right = math.ceil(view * w / 2 + (-0.4 + 4) / 8 * w / 2) + 1
                    top = math.floor((6 + 5.35) / 12 * h) - 1
                    bottom = math.ceil((6 + 5.85) / 12 * h) + 1
                    changed = [(i * 4 // zdepth.pitch, (i * 4 % zdepth.pitch) // 4) for i, (a, b) in enumerate(zip(struct.iter_unpack("<f", zdepth.data), struct.iter_unpack("<f", before["depth"].data))) if a != b]
                    check(bool(changed) and all(top <= y <= bottom and left <= x <= right for y, x in changed), (name, view, "only deferred stock main may provide new depth", len(changed)))
                else:
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


def validate_readiness(name):
    directory = OUT / name
    log = (directory / "run.log").read_text()
    check("epoch=360" in log and "plan_ready=false draw_ready=true split_planned=false" in log, (name, "actual post-plan first readiness transition"))
    check("plan_ready=true draw_ready=true split_planned=true" in log, (name, "next-frame split becomes ready"))
    for view in (0, 1):
        base = None
        for frame in (360, 361):
            rows = list(csv.DictReader((directory / f"v{view}-f{frame}-readiness.csv").open()))
            check([r["stage"] for r in rows] == ["z", "motion"], (name, view, frame, "both stages reached"))
            for row in rows:
                check(row["ready_before_draw"] == "true", (name, view, frame, "cache base ready before draw"))
                check(row["ready_at_plan"] == str(frame != 360).lower(), (name, view, frame, "actual cache readiness at plan"))
                check(row["split_planned"] == str(frame != 360).lower(), (name, view, frame, "split planning"))
                if frame == 360:
                    check(row["queued_pipeline"] == row["base_pipeline"], (name, view, "first frame retains base queue identity"))
                    check(row["selected_pipeline"] == "" and row["selected_depth_writes"] == "" and row["submitted"] == "false", (name, view, row["stage"], "unplanned base must Skip even after becoming ready"))
                    base = row["base_pipeline"]
                else:
                    check(row["base_pipeline"] == base, (name, view, "same base retained for future planning"))
                    check(row["queued_pipeline"] != base, (name, view, "retained queue adopts split identity"))
                    check(row["selected_pipeline"] != base and row["selected_pipeline"] != "", (name, view, "original never replayed"))
                    check(row["selected_depth_writes"] == str(row["stage"] == "z").lower(), (name, view, "motion pipeline never writes depth"))
                    check(row["submitted"] == "true", (name, view, row["stage"], "actual trailing DrawMesh recorder"))
            check(rows[0]["queued_pipeline"] == rows[1]["queued_pipeline"], (name, view, frame, "same retained phase item reused"))
            if frame == 361:
                check(rows[0]["selected_pipeline"] == rows[0]["queued_pipeline"] and rows[1]["selected_pipeline"] != rows[0]["queued_pipeline"], (name, view, "Z then motion split dispatch"))
            before = Attachment(directory, view, frame, "before-motion", "motion")
            after = Attachment(directory, view, frame, "motion", "motion")
            check(before.sample(view, -1.0, -5.6) == (0.125, -0.25), (name, view, frame, "no early original motion write"))
            check(after.sample(view, -1.0, -5.6) == ((0.125, -0.25) if frame == 360 else (0.0, 0.0)), (name, view, frame, "deferred then actual split motion"))
            depth = Attachment(directory, view, frame, "motion", "depth")
            check(depth.data == Attachment(directory, view, frame, "before-motion", "depth").data, (name, view, frame, "read-only motion leaves depth unchanged"))
            normal = Attachment(directory, view, frame, "z", "normal").sample(view, -1.0, -5.6)[0] & 0x3fffffff
            check((normal != 0) == (frame == 361), (name, view, frame, "unplanned Z deferred; planned Z resumes"))


def validate_no_attachments(captures):
    if "none-split" not in captures:
        return
    directory = OUT / "none-split"
    expected = "view_prepass_textures=false\nopaque_prepass_phase=false\nalpha_prepass_phase=false\n"
    for view in (0, 1):
        for frame in (360, 361):
            check((directory / f"v{view}-f{frame}-no-attachments-state.txt").read_text() == expected, ("none", view, frame, "actual missing stock prepass resources"))
            depth = Attachment(directory, view, frame, "no-attachments", "depth").sample(view, -1.0, -5.6)[0]
            colour = Attachment(directory, view, frame, "no-attachments", "colour").sample(view, -1.0, -5.6)
            check(depth > 0 and colour[:3] == (0, 0, 255), ("none", view, frame, "stock main retains depth and color"))
    check(not list(directory.glob("*-normal.txt")) and not list(directory.glob("*-motion.txt")), "none must not manufacture normal/motion attachments")
    original = Image.open(OUT / "none-stock/shot.png").convert("RGB")
    split = Image.open(directory / "shot.png").convert("RGB")
    check(original.tobytes() == split.tobytes(), "no-attachments stock material/shadow/main screenshot equality")


def main():
    captures = json.loads((OUT / "captures.json").read_text())
    for name, report in captures.items():
        for filename, digest in report["files"].items():
            data = attachment_bytes(OUT / name, filename)
            check(hashlib.sha256(data).hexdigest() == digest, (name, filename, "retained hash"))
        if "attachment_archive_sha256" in report:
            check(hashlib.sha256((OUT / name / "attachments.zip").read_bytes()).hexdigest() == report["attachment_archive_sha256"], (name, "archive hash"))
        if name in ("bindless", "ordinary", "depth", "normal", "motion", "noz-motion", "targets", "occlusion", "readiness", "readiness-ordinary"):
            validate_case(name, report)
            if name.startswith("readiness"):
                validate_readiness(name)
            print("PASS", name)
    validate_no_attachments(captures)
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
    if not ARGS.read_only:
        historical = ROOT / "data/extract/aa-cry/native-pass-probe-20261024"
        assert not OUT.resolve().is_relative_to(historical.resolve()), "use --read-only for historical evidence"
        (OUT / "validation.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
