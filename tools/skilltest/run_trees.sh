#!/bin/sh
# usage: [EXTRA="--at ..."] tools/skilltest/run_trees.sh TAG TREE...   (one process per tree; results in data/extract/skilltest/TAG_treeN.jsonl/.log)
tag=$1; shift
for t in "$@"; do
  tools/skilltest/retry.sh data/extract/skilltest/${tag}_tree$t --trees $t $EXTRA
done
