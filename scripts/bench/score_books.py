#!/usr/bin/env python3
"""Scores every engine on the book-layout corpus (tests/corpus/books/pdf),
against the ground truth written by scripts/make_books.py.

    python3 scripts/bench/score_books.py [--engines akkhor,mupdf,...] [--csv out.csv]

Page text, every engine:
  words_intact  share of the truth's words that appear as whole words
                (a word run into its neighbour, or split by a line-end
                hyphen, is not intact: double-tap selection, read-aloud and
                word counts break on it)
  words_found   share found as substrings, the search test of the case study
Reflow, Akkhor only (`akkhor-bench reflow`):
  reflow_cer    character error rate of the reflowed text against the truth
                blocks, whitespace ignored
  starts_p/r    precision and recall of block starts (headings, paragraphs,
                footnotes)
  headings      share of truth headings found as headings with the right level
  furniture     running headers and page numbers that leaked into the reflow
  footnotes     share of truth footnotes found as footnote blocks (they are
                left out of reflow_cer: after the paragraph or at the end of
                the page are both fine)
  images        image blocks found minus image blocks in the truth
"""

import argparse
import csv
import json
import os
import re
import sys
import unicodedata

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from compare_engines import ENGINES, call, edit_distance, normal  # noqa: E402

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
BOOKS = os.path.join(ROOT, "tests", "corpus", "books", "pdf")
SPLIT = r"[\s,।॥:;()\-!?\"“”‘’'·.]+"


def tokens(text):
    return [unicodedata.normalize("NFC", w) for w in re.split(SPLIT, text) if w]


def truth_words(t):
    ws = []
    for b in t["blocks"]:
        ws += [w for w in tokens(b["text"]) if len(w) >= 2]
    return ws


def score_page_text(t, pages):
    text = "\n".join(pages)
    have = set(tokens(text))
    ws = truth_words(t)
    spaced = " ".join(unicodedata.normalize("NFC", text).split())
    return {
        "words_intact": sum(w in have for w in ws) / max(1, len(ws)),
        "words_found": sum(w in spaced for w in ws) / max(1, len(ws)),
    }


def starts(blocks):
    """The first few letters of each text block: a block start, matched by
    position-independent key so reordering shows up in the CER instead."""
    out = []
    for b in blocks:
        n = normal(b.get("text", ""))
        if n:
            out.append(n[:12])
    return out


def score_reflow(t, r):
    # Footnotes are compared on their own: a reflow may put them after
    # their paragraph or at the end of the page, both readable.
    truth_blocks = [b for b in t["blocks"] if b["kind"] not in ("image", "footnote")]
    got = [b for b in r["blocks"] if b["kind"] not in ("image", "footnote")]
    t_notes = [normal(b["text"]) for b in t["blocks"] if b["kind"] == "footnote"]
    g_notes = [normal(b["text"]) for b in r["blocks"] if b["kind"] == "footnote"]
    # Footnote markers are part of the paragraph text in producers with
    # footnotes; the truth does not have them, so digits right after a
    # paragraph's last letter are dropped before comparing.
    def clean(s):
        return re.sub(r"(?<=[^\d\s])[0-9০-৯०-९]+(?=\s|$)", "", s)
    t_all = normal("".join(b["text"] for b in truth_blocks))
    g_all = normal(clean("".join(b["text"] for b in got)))
    cer = edit_distance(t_all, g_all) / max(1, len(t_all))
    ts, gs = starts(truth_blocks), starts([{"text": clean(b["text"])} for b in got])
    hit = sum(1 for s in gs if s in ts)
    heads = [b for b in truth_blocks if b["kind"] == "heading"]
    got_heads = {(normal(b["text"]), b.get("level")) for b in got if b["kind"] == "heading"}
    # Furniture: the running header, or a block that is only a page number.
    leaks = 0
    for b in got:
        txt = b["text"].strip()
        if any(normal(f) in normal(txt) for f in t["furniture"]):
            leaks += 1
        elif re.fullmatch(r"[0-9০-৯०-९ivxlcIVXLC]+", txt):
            leaks += 1
    return {
        "reflow_cer": cer,
        "starts_p": hit / max(1, len(gs)),
        "starts_r": sum(1 for s in ts if s in gs) / max(1, len(ts)),
        "headings": sum(1 for h in heads if (normal(h["text"]), h["level"]) in got_heads) / max(1, len(heads)),
        "furniture": leaks,
        "footnotes": sum(1 for n in t_notes if any(n in g for g in g_notes)) / len(t_notes) if t_notes else 1.0,
        "images": sum(1 for b in r["blocks"] if b["kind"] == "image") - sum(1 for b in t["blocks"] if b["kind"] == "image"),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engines", default="akkhor,mupdf,pdfium,poppler")
    ap.add_argument("--csv")
    args = ap.parse_args()
    engines = args.engines.split(",")
    rows = []
    for name in sorted(os.listdir(BOOKS)):
        if not name.endswith(".pdf"):
            continue
        path = os.path.join(BOOKS, name)
        t = json.load(open(path[:-4] + ".json", encoding="utf-8"))
        for e in engines:
            out, _ = call(e, "text", path)
            row = {"file": name, "engine": ENGINES[e]}
            row.update(score_page_text(t, out["pages"] if out else []))
            if e.startswith("akkhor"):
                r, code = call(e, "reflow", path)
                if r and "blocks" in r:
                    row.update(score_reflow(t, r))
            rows.append(row)
            cells = " ".join(f"{k} {v:.3f}" if isinstance(v, float) else f"{k} {v}"
                             for k, v in row.items() if k not in ("file", "engine"))
            print(f"{name:32} {row['engine']:8} {cells}")
    if args.csv:
        keys = sorted({k for r in rows for k in r}, key=lambda k: (k not in ("file", "engine"), k))
        with open(args.csv, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, keys)
            w.writeheader()
            w.writerows(rows)
    # Summary per engine.
    print()
    for e in engines:
        rs = [r for r in rows if r["engine"] == ENGINES[e]]
        summary = {k: sum(r[k] for r in rs if k in r) / max(1, sum(1 for r in rs if k in r))
                   for k in ("words_intact", "words_found", "reflow_cer", "starts_p", "starts_r", "headings", "footnotes")
                   if any(k in r for r in rs)}
        leaks = sum(r.get("furniture", 0) for r in rs)
        print(f"{ENGINES[e]:8} " + " ".join(f"{k} {v * 100:.1f}%" for k, v in summary.items())
              + (f" furniture leaks {leaks}" if any("furniture" in r for r in rs) else ""))


if __name__ == "__main__":
    main()
