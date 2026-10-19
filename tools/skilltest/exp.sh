#!/bin/sh
# follow-up experiments after the tree passes (one driver process at a time, same character)
AT="--at 15530,15323,131.4"
cp target/debug/examples/skilltest.exe target/debug/examples/skilltest_run.exe
tools/skilltest/retry.sh data/extract/skilltest/e1_hostile_dummy --npc 7512 $AT --skills 14376,10154,11933,10159,10534,18132
tools/skilltest/retry.sh data/extract/skilltest/e2_cooldowns --keep-cooldowns --repeat $AT --skills 18132,11918,16210,10667,10534,14495
tools/skilltest/retry.sh data/extract/skilltest/e3_revive_corpse --corpse $AT --skills 10546
tools/skilltest/retry.sh data/extract/skilltest/e4_unlearned --no-learn $AT --skills 12026,16210
