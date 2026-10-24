# Original Illum pass inputs

The original renderer has distinct per-object alpha thresholds for forward and
depth work. Motion eligibility also depends on the current renderer state,
technique and object flags. A material cutoff or a camera prepass marker does not
supply all of these inputs.

The [standalone verifier](../../tools/native_illum_inputs.py) executes **477
bounded original-instruction controls** against the retained 32-bit
`cryrenderd3d10.dll`. All outputs match the
[retained cases](../../tools/evidence/illum_inputs/native-cases.json), including
raw vector bytes and executed-address trace hashes. This is input and branch
evidence, not a native scene, driver, rendered-image or performance comparison.
Production shaders and camera policy were unchanged for this audit.

## Alpha thresholds

Original `380733b0` supplies the PI vector; `38073510` supplies PB. The 60
controls cover material absence, object bytes, sign, forced threshold, shadow
state, list selection and both valid dynamic lookup tables.

| Input | PI | PB |
|---|---|---|
| `.x` | Object byte `+98` times the native f32 `1/255` constant | Zero |
| `.y` | `+1` when object flags contain `0x02000000`, otherwise `-1` | Same |
| Base `.w` | Material float `+20`, or zero without material | Same |
| Shader flags `+40 & 0x800000`, outside shadow thread state | Replace base with native f32 `0.51` | Same |
| Object byte `+99` | Add byte times the native f32 `1/255` constant; no clamp | No addition |
| Object `+100 & 0x100`, list 7 or 17, valid indexed row | Replace `.w` with row float `+48`, after the preceding operations | Same |

An existing first value of zero is a value, not a missing input. Thresholds can
exceed one after PI bias. These tests use valid dynamic indices; invalid native
indices can reach a null-derived read and are not converted into a fabricated
native refusal.

PB's `.z` has a separate SSDO time path. Static registration ties `383b12c4` to
`r_ssdo` and `383b12b4` to `r_ssdoAmount`; the enabled branch calls the timer and
multiplies by that amount. The alpha fixtures explicitly disable this path.
They do not establish timer behavior or the complete PB vector under enabled
SSDO.

## Motion and frame state

The 384 controls execute the complete original qualifier `38178cd0`. In the
audited ordinary recursion-1 path, motion batch bit `0x100` requires all of:

- Signed technique byte `+35` is positive.
- The object's first DWORD has its high bit set.
- Effective renderer value `383b1284` is nonzero.
- Shader flags `+36` do not contain `0x80`.
- Object flags do not contain any bit in `0x40000400`.

The matrix includes opaque and reduced material opacity, recursion 1/2, signed
alias boundaries, zero/nonzero renderer values, and each suppressing object
flag. Material opacity alone does not decide motion admission. These controls
do not assign an unproved semantic name to the object's high bit or establish
its application-side producer.

Twelve complete `3817d100` controls show configured `383b1294` copied to effective
`383b1284` with either no post-effect manager or its empty vector. Other
post-effect branches are outside that fixture. Twenty scene controls execute
the original save/gate and restore slices: scene flag `0x40` temporarily clears
the effective value, and the saved value is restored afterward. Intervening
scene draws were not executed.

One original registration slice calls a controlled console sink with
`r_MotionBlur`, storage `383b1294`, and requested default **zero**. Its help text
mentions a different default. Neither the sink nor the static registration
establishes the eventual configured value after profile/config hydration.

## Reproduction and limits

From the repository root, with Python `pefile` and `unicorn` available:

```powershell
python -B tools/native_illum_inputs.py
# An explicitly selected retained copy is also accepted, only with the same hash:
python -B tools/native_illum_inputs.py --dll <retained-original-renderer.dll>
```

Required module SHA-256:
`ebf051bba5d349a6b3bf6d4eb676908115b73bb22d35d5a0cc917c325426589d`.
The default path is the existing ignored
`data/extract/aa-cry/ida/render/cryrenderd3d10.dll`. No DLL is loaded into the host
process. Each invocation has a two-second/50,000-instruction bound and an
execution-range allowlist. Only the registration console sink is substituted;
the other selected functions/slices have no substituted call endpoints. Fixture
objects and state are supplied explicitly. The verifier writes no files and
checks the module hash again after execution.

The new static disassembly, decompilation and provenance remain in ignored
`data/extract/aa-client/illum-instance-inputs-20261024/`. Existing accepted
renderer script, goldens, original module and IDB hashes were unchanged. A
task-owned IDB copy was used without saving to the original database.

Successful-Z recording, paired forward readiness, independent motion coverage,
per-instance data transport, authoritative view policy and authored sampler
integration remain production prerequisites. The current masked shader's
historical main/prepass equality checks do not prove those native rules.
