# World look: the 1.2 client's stills against aa-client's

Stills of the original client and aa-client from the same camera at the same hour, and the numbers that compare them. The
result of the first round (sun against ambient) is in `docs/aa-world.md`, "Sun against ambient".

## The original

`orig.py` drives a running 1.2 client through the client driver of the skill parity tools (`AA_DRIVER` / `AA_DRIVER_DLL`; the
driver needs the `f12` key of `client_input`).

1. Start it in the world (`tools/skillparity/launch_original.py --character NAME --visible --pid-file PID`).
2. Load it at spec 4. The saved graphics options load the low preset whatever `system.cfg`'s `sys_spec_full` says, and
   `orig.py spec 4` at run time does not reload the vegetation: the grass and bushes of min spec 2 and 3 stay missing. After
   `spec 4`, a GM `/move` 5 km away and back (`client_chat`; from Git Bash set `MSYS_NO_PATHCONV=1`, or `/move` becomes a
   path) loads the cells again at spec 4.
3. `orig.py f12` hides the UI (a teleport shows it again).
4. `orig.py --player NAME --origin X,Y shot OUT.png` per still (turn with `orig.py key d 450`). The state JSON next to it holds
   the camera, the hour and the engine's lighting values. `--origin` is the seamless origin of the client's local frame; it
   moves with a teleport (after `/move 15530 15325 ...` it is 11264,11264).

The game rewrites `Documents/ArcheAge/system.cfg` when it exits: keep a copy and put it back.

## aa-client

`stills.py OUT.txt SHOTS TAG STATE.json...` writes the script that takes the same stills (`hour`, `face`, `view`, `shot`);
run aa-client with `--script OUT.txt --no-hud --cam-fov 57`, `AA_TUNE=...` for atmosphere settings. The original and aa-client
cannot be in the world on the same account at once: close the original first.

## Comparing

`compare.py fit ORIG OURS ...` (luminance, gain and hue of the sky and the ground), `compare.py shadow LIT DIM` (how much
light shade keeps) and `compare.py sheet` (side by side). `crates/aa-world/tools/lighting_fit.py` keeps the older reference
views honest (`AA_FLY`, `AA_REF` point it at a build and the local references).
