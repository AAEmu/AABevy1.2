#!/bin/sh
# usage: tools/skilltest/client_sample.sh NAME TREE SHOT_AFTER SKILL1 SKILL2 SKILL3 SKILL4
# learns the tree on the account with the driver, then runs the real client (a copy in target/agents/skilltest/): target the nearest
# enemy at 14 s, slot 1..4 at 16, 20, 24, 28 s, one screenshot, log in shots/skilltest/NAME.log. Check free RAM first.
name=$1; tree=$2; after=$3; shift 3
tools/skilltest/retry.sh data/extract/skilltest/learn_$name --trees $tree --learn-only --at 15530,15323,131.4 > /dev/null 2>&1
sleep 5
MSYS_NO_PATHCONV=1 AA_COMBAT_LOG=1 timeout 300 ./target/agents/skilltest/aa-client.exe --enter-world --account bevyskilltest --character Skilltestr \
  --hour 12 --pos 15530,15323 --yaw 0 --cam-dist 6 --cam-pitch -18 --bar-skill 1 $1 --bar-skill 2 $2 --bar-skill 3 $3 --bar-skill 4 $4 \
  --hud-key tab 14 --hud-key slot1 16 --hud-key slot2 20 --hud-key slot3 24 --hud-key slot4 28 \
  ${SHOTARGS:---shot shots/skilltest/$name.png --shot-after $after} > shots/skilltest/$name.log 2>&1
