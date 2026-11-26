# Akkhor on a phone

The engine's numbers so far come from a cloud VM. This measures the same
things on a real phone: open, the library card (info and cover), the
first screen, a redraw, a chapter as reflowed text, search, and memory.

## Get it

From the latest green CI run on GitHub (Actions, the "CI" workflow), download
the artefact **akkhor-bench-arm64** and unpack it. It holds `akkhor-bench`
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

**On the phone alone (Termux):** copy the folder to the phone, then in
Termux: `termux-setup-storage`, copy the folder into Termux's home
(`cp -r /sdcard/Download/akkhor-bench-arm64 ~/ab`), and run
`sh ~/ab/on-phone.sh`.

Your own books can be added at the end of either command
(`sh run.sh /sdcard/Download/book.pdf`); they appear as `own-1`, `own-2`
in the results, so no title leaves the phone.

## Send back

`results.jsonl` (a few KB). It says the phone model and Android version,
then one line per book and pass. Each pass is a new process, like the
app starting; "cold" is the first, when the file is not yet in the
phone's file cache, "warm" the two after. The numbers go into
`docs/case-study/data/phone.csv` next to the VM numbers.
