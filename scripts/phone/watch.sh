#!/data/data/com.termux/files/usr/bin/sh
# Runs on the phone, in Termux, and keeps going: when CI finishes a build
# whose commit title contains [phone], downloads that build's phone bundle,
# runs it (on-phone.sh) and sends results.jsonl back to the repository, on
# the branch phone-results, as results/<artifact tag>.jsonl.
#
# Once:   pkg install gh
#         gh auth login            (GitHub.com, HTTPS, log in with a browser)
# Then:   sh watch.sh              (checks every 5 minutes; Ctrl+C to stop)
#         sh watch.sh 60           (every minute)
#         sh watch.sh once         (check now, run if there is a new build, stop)
#
# Keep the phone charging. The screen may be off: Termux holds a wake lock
# while this runs. Nothing but the bench results leaves the phone: the
# device model, Android version and timings, no book titles of your own.
set -u
repo=MisbahKhan0009/akkhor
dir=${AKKHOR_PHONE_DIR:-$HOME/akkhor-phone}
mode=${1:-300}
mkdir -p "$dir" && cd "$dir" || exit 1
command -v gh >/dev/null || { echo "gh is missing: pkg install gh"; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "log in first: gh auth login"; exit 1; }
termux-wake-lock 2>/dev/null || true

check() {
    # The newest successful CI build asked to run on the phone.
    run=$(gh run list -R $repo --workflow CI --branch main --status success --limit 20 \
        --json databaseId,displayTitle \
        --jq '[.[] | select(.displayTitle | contains("[phone]"))][0].databaseId // empty')
    [ -n "$run" ] || return 0
    [ "$run" = "$(cat done-run 2>/dev/null)" ] && return 0
    name=$(gh api repos/$repo/actions/runs/$run/artifacts \
        --jq '.artifacts[] | select(.name | startswith("akkhor-bench-arm64")) | .name' | head -1)
    [ -n "$name" ] || return 0
    echo "$(date '+%F %T') run $run: $name"
    rm -rf bundle
    gh run download "$run" -R $repo -n "$name" -D bundle || return 0
    (cd bundle && sh on-phone.sh > on-phone.log 2>&1)
    [ -s bundle/results.jsonl ] || { echo "no results; see bundle/on-phone.log"; return 0; }
    tag=${name#akkhor-bench-arm64-}
    # An existing file is replaced (its sha is needed for that).
    sha=$(gh api "repos/$repo/contents/results/$tag.jsonl?ref=phone-results" --jq .sha 2>/dev/null || true)
    set -- -X PUT "repos/$repo/contents/results/$tag.jsonl" \
        -f message="Phone results $tag" -f branch=phone-results \
        -f content="$(base64 -w0 bundle/results.jsonl)"
    [ -n "$sha" ] && set -- "$@" -f sha="$sha"
    if gh api "$@" >/dev/null; then
        echo "$run" > done-run
        echo "$(date '+%F %T') sent results/$tag.jsonl"
    else
        echo "sending failed; results kept in $dir/bundle/results.jsonl"
    fi
}

if [ "$mode" = once ]; then
    check
else
    echo "watching $repo every $mode s (Ctrl+C to stop)"
    while :; do
        check
        sleep "$mode"
    done
fi
