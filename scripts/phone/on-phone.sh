#!/system/bin/sh
# Runs on the phone (adb shell or Termux), from the folder holding
# akkhor-bench and the books. Writes results.jsonl beside it.
#   sh on-phone.sh [EXTRA_BOOK ...]
# Extra books (your own) are named own-1, own-2 ... in the results, so
# no title leaves the phone.
here=$(pwd)
cd "$(dirname "$0")" || exit 1
chmod 755 akkhor-bench
out=results.jsonl
: > $out
note() { echo "$1" | tee -a $out; }
note "{\"device\":\"$(getprop ro.product.model 2>/dev/null)\",\"android\":\"$(getprop ro.build.version.release 2>/dev/null)\",\"cpu\":\"$(getprop ro.soc.model 2>/dev/null)\",\"abi\":\"$(getprop ro.product.cpu.abi 2>/dev/null)\"}"
run() { # label file
    for pass in cold warm warm; do
        line=$(./akkhor-bench phone "$2" 2>&1 | sed "s#\"file\":\"[^\"]*\"#\"file\":\"$1\",\"pass\":\"$pass\"#")
        note "$line"
    done
}
for f in books/*; do run "$(basename "$f")" "$f"; done
i=0
for f in "$@"; do
    case "$f" in /*) ;; *) f="$here/$f" ;; esac
    i=$((i + 1)); run "own-$i" "$f"
done
# The library screen: every book's card in one process, twice.
note "{\"library\":[$(./akkhor-bench library books/* | paste -sd, -)]}"
if [ -f books/big.pdf ]; then
    note "{\"big_last_page\":$(./akkhor-bench speed books/big.pdf 1999 2.5 3)}"
    note "{\"big_session_100_pages\":$(./akkhor-bench session books/big.pdf 100 2.5)}"
fi
echo "done: $(pwd)/$out"
