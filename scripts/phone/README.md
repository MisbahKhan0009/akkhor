# Akkhor on a phone

The engine's numbers so far come from a cloud VM. This measures the same
things on a real phone: open, the library card (info and cover), the
first screen, a redraw, a chapter as reflowed text, search, and memory.

## Get it

From the latest green CI run on GitHub (Actions, the "CI" workflow), download
the artefact **akkhor-bench-arm64-DATE-runN-COMMIT** (each build's name is
its own, so downloads never clash) and unpack it. It holds `akkhor-bench`
(arm64 Android), `on-phone.sh`, `run.sh` and `books/` (the test books).

Optional, for the big-file numbers: on a computer with the repository,
`python3 scripts/make_fixtures.py --big 2000 --image-kb 150`, then copy
`tests/fixtures/big.pdf` (about 300 MB) into `books/`.

## Run it

**With a computer (adb):** turn on USB debugging on the phone, connect it,
then in the unpacked folder:

```sh
sh run.sh
```

**On the phone alone (Termux):** download the zip on the phone, then:

```sh
termux-setup-storage
cd ~ && rm -rf ab && mkdir ab && cd ab
unzip "$(ls -t ~/storage/downloads/akkhor-bench-arm64-*.zip | head -1)"
sh on-phone.sh
cp results.jsonl ~/storage/downloads/
```

**Automatically (Termux, no downloads):** `watch.sh` waits for CI builds
whose commit title contains `[phone]`, runs each one's bundle and sends
`results.jsonl` back to the repository (branch `phone-results`, file
`results/DATE-runN-COMMIT.jsonl`), where the engine work picks it up.
Once:

```sh
pkg install gh
gh auth login                      # GitHub.com, HTTPS, log in with a browser
curl -LO https://raw.githubusercontent.com/MisbahKhan0009/akkhor/main/scripts/phone/watch.sh
```

Then, with the phone charging and its screen on (a phone with the screen
off slows down, and runs stop being comparable; Developer options, "Stay
awake" keeps it on while charging): `sh watch.sh` (checks every 5 minutes;
Ctrl+C stops it), or `sh watch.sh once` to check one time.

Your own books can be added at the end of either command
(`sh run.sh /sdcard/Download/book.pdf`); they appear as `own-1`, `own-2`
in the results, so no title leaves the phone.

## Send back

`results.jsonl` (a few KB; `watch.sh` sends it by itself). It says the phone model and Android version,
then one line per book and pass. Each pass is a new process, like the
app starting; "cold" is the first, when the file is not yet in the
phone's file cache, "warm" the two after. The numbers go into
`docs/case-study/data/phone.csv` next to the VM numbers.
