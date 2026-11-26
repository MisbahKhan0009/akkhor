#!/bin/sh
# From a computer with adb and a phone in USB debugging mode, in the
# unpacked akkhor-bench-arm64 folder (CI artefact):
#   sh run.sh [/sdcard/Download/my-book.pdf ...]
# Copies the bench and test books to the phone, runs them, and brings
# results.jsonl back here.
set -e
d=/data/local/tmp/akkhor
adb shell "rm -rf $d && mkdir -p $d"
adb push akkhor-bench on-phone.sh $d/ >/dev/null
adb push books $d/ >/dev/null
adb shell "sh $d/on-phone.sh $*"
adb pull $d/results.jsonl . >/dev/null
echo "results.jsonl is here; send it over."
