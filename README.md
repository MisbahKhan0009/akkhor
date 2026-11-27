# Phone results

Written by `scripts/phone/watch.sh` running in Termux on the test phone:
one `results/DATE-runN-COMMIT.jsonl` per CI build whose commit title
contains `[phone]`. Each file is that build's `akkhor-bench` run (see
`scripts/phone/README.md` on `main`). Only the phone model, Android
version and timings are in them; no book titles of the owner's.
