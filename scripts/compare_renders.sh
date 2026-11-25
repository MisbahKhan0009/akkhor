#!/usr/bin/env bash
# Renders pages of every fixture and corpus PDF with two akkhor binaries and
# reports any difference in page count or pixels. Used when changing the
# vendored PDF parser:  scripts/compare_renders.sh OLD_BINARY NEW_BINARY
set -u
old=$1 new=$2 diffs=0
for f in tests/fixtures/*.pdf tests/corpus/pdf/*.pdf; do
  for p in 0 1; do
    a=$("$old" "$f" --page $p --scale 0.5 --out /tmp/cmp-a.png 2>&1 | grep -E "^pages|^error")
    b=$("$new" "$f" --page $p --scale 0.5 --out /tmp/cmp-b.png 2>&1 | grep -E "^pages|^error")
    if [ "$a" != "$b" ] || ! cmp -s /tmp/cmp-a.png /tmp/cmp-b.png; then
      echo "DIFF $f page $p: [$a] vs [$b]"; diffs=$((diffs + 1))
    fi
  done
done
echo "$diffs differences"
[ "$diffs" = 0 ]
