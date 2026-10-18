# Skill reel: baseline

Status (2026-10-18): tooling ready, recording blocked by the lane server (see "Blocker"). No videos yet.

## Frozen baseline build

* `target/agents/reel/baseline/aa-client.exe`, git `77d3f01` plus the uncommitted tree of that moment (`uncommitted.diff`,
  `status.txt`, `NOTES.txt` next to it: other sessions' uncommitted aa-anim / aa-cry / aa-world / aa-net edits are in it).
* Built from a source snapshot in a second target dir (`D:\aabevy-reel`, hard-linked deps via `tools/reel/linkfarm.py`), because the
  shared tree already carried the skill-fix edits. The same trick builds `after` without touching the shared target.

## Tooling

* `aa-dev --record <dir> [--record-fps 30] [--record-from S] [--record-secs N] [--record-gate]`: PNG per frame, fixed timestep
  (`TimeUpdateStrategy::ManualDuration`). Tested on the login screen: 90 frames of 1280x720 in 0.5 s wall, about 1.4 MB per PNG.
* `AA_COMBAT_FAKE_TARGET=<obj id | model=ID | name part>` aims the fake skill events at another unit (the training dummy is
  `model=1290`, npc 7512, spawned by `/spawn npc dummy`); an op suffix `@self` (`fire@self`) keeps one event on the player. Fake
  time 0 opens `aa_dev::RecordGate`, so `--record-from` counts from the moment the player appeared.
* `tools/reel/plan.py` writes `data/extract/reel/plans/NN_<ability>.json` (110 shown tree skills, weapon per tree, captions);
  `tools/reel/run_reel.py baseline|after|compare` records, captions (ffmpeg drawtext), builds contact sheets and the mp4s.
  ffmpeg: `data/extract/reel/venv` (imageio-ffmpeg).

## Blocker

Every freshly created character hangs in `CSSelectCharacter` on the `bevy` lane (the client gets no answer, the walkthrough
example times out after 20 s, and the account then stays "online": `enter world denied (1)`). Five fresh accounts (mine and
other sessions') behave the same since about 19:08. The game server's `select_character` takes `server.save.transaction_lock()`
first; a stuck holder (the lane has "Players online: 2" ghosts from yesterday) blocks every later select. The lane's game
server needs a restart (`Scripts/live-stack.sh stop/start` in `D:\aaemu-wt\bevy`) before any in-game recording works.
