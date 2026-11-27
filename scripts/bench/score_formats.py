#!/usr/bin/env python3
"""Scores Akkhor, MuPDF, KindleUnpack and Calibre on the Phase 6 format
corpus (tests/corpus/formats), against the text of the EPUB each book was
converted from (scripts/make_format_corpus.py).

    python3 scripts/bench/score_formats.py [--csv out.csv]

Per book and engine:
  words_in_order  share of the truth's words (two letters or more) found in
                  the engine's text in the same order (longest common run
                  of words, difflib)
  words_found     share found anywhere as whole words
  extra           words in the engine's text that are not in the truth, per
                  truth word (repeated titles, leaked markup, page furniture)
  ms              wall time to open the book and get all its text
An engine that can't open the book scores empty.

Akkhor runs `target/release/examples/booktext` (build it first:
`cargo build --release -p akkhor --example booktext`).
"""

import argparse
import csv
import difflib
import glob
import html
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unicodedata
import warnings
import zipfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
FORMATS = os.path.join(ROOT, "tests", "corpus", "formats")
SPLIT = r"[\s,।॥:;()\-!?\"“”‘’'·.*]+"


def tokens(text):
    return [unicodedata.normalize("NFC", w) for w in re.split(SPLIT, text) if w]


def html_text(markup):
    markup = re.sub(r"(?is)<(script|style|head)\b.*?</\1>", " ", markup)
    markup = re.sub(r"(?i)<br\s*/?>|</(p|div|h[1-6]|li|tr)>", "\n", markup)
    return html.unescape(re.sub(r"(?s)<[^>]+>", " ", markup))


def epub_text(path):
    """Spine documents in order, as text."""
    z = zipfile.ZipFile(path)
    container = z.read("META-INF/container.xml").decode()
    opf_path = re.search(r'full-path="([^"]+)"', container).group(1)
    opf = z.read(opf_path).decode()
    base = os.path.dirname(opf_path)
    items = dict(re.findall(r'<item\b[^>]*\bid="([^"]+)"[^>]*\bhref="([^"]+)"', opf))
    items.update({i: h for h, i in re.findall(r'<item\b[^>]*\bhref="([^"]+)"[^>]*\bid="([^"]+)"', opf)})
    out = []
    for idref in re.findall(r'<itemref\b[^>]*\bidref="([^"]+)"', opf):
        href = items.get(idref)
        if not href or "nav" in href.lower():
            continue
        out.append(html_text(z.read(os.path.normpath(os.path.join(base, href))).decode("utf-8", "replace")))
    return "\n".join(out)


def akkhor(path):
    exe = os.path.join(ROOT, "target", "release", "examples", "booktext")
    r = subprocess.run([exe, path], capture_output=True, timeout=120)
    return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None


def mupdf(path):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        import pymupdf
    try:
        d = pymupdf.open(path)
        return "\n".join(p.get_text() for p in d)
    except Exception:
        return None


def kindleunpack(path):
    if not path.endswith((".mobi", ".azw3")):
        return None
    import mobi
    try:
        tmp, out = mobi.extract(path)
    except Exception:
        return None
    try:
        if out.endswith(".epub"):
            return epub_text(out)
        with open(out, "rb") as f:
            return html_text(f.read().decode("utf-8", "replace"))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def calibre(path):
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "out.txt")
        r = subprocess.run(["ebook-convert", path, out], capture_output=True, timeout=300)
        if r.returncode != 0 or not os.path.exists(out):
            return None
        with open(out, encoding="utf-8", errors="replace") as f:
            return f.read()


ENGINES = {"akkhor": akkhor, "mupdf": mupdf, "kindleunpack": kindleunpack, "calibre": calibre}


def score(truth, text):
    want = [w for w in tokens(truth) if len(w) >= 2]
    if text is None:
        return {"words_in_order": 0.0, "words_found": 0.0, "extra": 0.0}
    got = [w for w in tokens(text) if len(w) >= 2]
    sm = difflib.SequenceMatcher(None, want, got, autojunk=False)
    in_order = sum(b.size for b in sm.get_matching_blocks())
    have, known = set(got), set(want)
    return {
        "words_in_order": in_order / max(1, len(want)),
        "words_found": sum(w in have for w in want) / max(1, len(want)),
        "extra": sum(w not in known for w in got) / max(1, len(want)),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--csv")
    ap.add_argument("--engines", default=",".join(ENGINES))
    args = ap.parse_args()
    truths = {lang: epub_text(os.path.join(FORMATS, "src", f"{lang}.epub")) for lang in ("bn", "en", "hi")}
    rows = []
    books = sorted(glob.glob(os.path.join(FORMATS, "*.mobi")) + glob.glob(os.path.join(FORMATS, "*.azw3"))
                   + glob.glob(os.path.join(FORMATS, "??.fb2")))
    for path in books:
        name = os.path.basename(path)
        lang = name[:2]
        for engine in args.engines.split(","):
            t = time.perf_counter()
            text = ENGINES[engine](path)
            ms = (time.perf_counter() - t) * 1e3
            s = score(truths[lang], text)
            row = {"book": name, "engine": engine, **{k: round(v, 4) for k, v in s.items()},
                   "ms": round(ms, 1), "opened": text is not None}
            rows.append(row)
            print(f"{name:16} {engine:13} order {s['words_in_order']:.3f} found {s['words_found']:.3f} "
                  f"extra {s['extra']:.3f} {ms:8.1f} ms", file=sys.stderr)
    if args.csv:
        with open(args.csv, "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)


if __name__ == "__main__":
    main()
