#!/usr/bin/env python3
"""Measures real-world PDFs that cannot be committed (copyrighted books),
on this machine only. Nothing from the PDFs is written to the repository;
the output is a few numbers per file.

    python3 scripts/bench/score_local.py FILE.pdf --fonts DIR [--skip N]

There is no hand-made ground truth for a whole book, so the reference is
Akkhor's own text with the book's fonts registered (`--fonts`, the fonts
the PDF embeds, as freely licensed originals). Spot-check a few pages of
that reference against the page images before trusting the numbers; the
comparison is by Bangla and Devanagari words (a bag of words, so reading
order does not count). `--skip` leaves out cover pages.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import unicodedata
from collections import Counter

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
BENCH = os.path.join(ROOT, "target", "release", "akkhor-bench")
ENGINES_PY = os.path.join(ROOT, "scripts", "bench", "engines.py")
WORD = re.compile(r"[ঀ-৿ऀ-ॿ‌‍]+")
LETTER = re.compile(r"[অ-হअ-ह]")


def pages(cmd, env=None):
    out = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=3600)
    return json.loads(out.stdout.strip().splitlines()[-1])["pages"]


def words(texts):
    c = Counter()
    for t in texts:
        c.update(w for w in WORD.findall(unicodedata.normalize("NFC", t)) if len(w) >= 2 and LETTER.search(w))
    return c


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pdf")
    ap.add_argument("--fonts", required=True)
    ap.add_argument("--skip", type=int, default=0)
    a = ap.parse_args()
    env = dict(os.environ, AKKHOR_REFERENCE_FONTS=a.fonts)
    ref = words(pages([BENCH, "text", a.pdf], env)[a.skip:])
    total = sum(ref.values())
    print(f"reference: {total} words ({len(ref)} distinct), {a.skip} pages skipped")
    runs = [("Akkhor (bundled fonts only)", [BENCH, "text", a.pdf])] + [
        (n, [sys.executable, ENGINES_PY, e, "text", a.pdf]) for n, e in
        (("MuPDF", "mupdf"), ("PDFium", "pdfium"), ("Poppler", "poppler"))]
    for name, cmd in runs:
        got = words(pages(cmd)[a.skip:])
        hit = sum(min(c, got[w]) for w, c in ref.items())
        print(f"{name:30} {100 * hit / max(1, total):5.1f}% of words intact")


if __name__ == "__main__":
    main()
