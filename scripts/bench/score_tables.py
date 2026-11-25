#!/usr/bin/env python3
"""Scores every engine on the table and card corpus (tests/corpus/tables/pdf),
against the truth written by scripts/make_tables.py.

    python3 scripts/bench/score_tables.py [--engines akkhor,mupdf,...] [--csv out.csv]

Page text, every engine:
  words_intact   share of the truth's words that come out as whole words
  rows_in_order  share of table rows (all tables, every row) whose cells
                 come out one after another, in order: a table read across,
                 not down its columns. Whitespace is ignored, so a row whose
                 cell wraps onto a second line still counts when the second
                 line follows.
  cards_whole    share of cards whose texts come out one after another
Reflow, Akkhor only (`akkhor-bench reflow`), the same three on the
reflowed text (a picture's alt text included, cell separators left out),
plus:
  text_rows      share of text-mode table rows that are one reflow block
  pictures       picture-mode tables shown as a picture (an image block)
"""

import argparse
import csv
import json
import os
import re
import sys
import unicodedata

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from compare_engines import ENGINES, call, normal  # noqa: E402

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
TABLES = os.path.join(ROOT, "tests", "corpus", "tables", "pdf")
SPLIT = r"[\s,।॥:;()\-!?\"“”‘’'·.]+"


def tokens(text):
    return [unicodedata.normalize("NFC", w) for w in re.split(SPLIT, text) if w]


def truth_texts(t):
    for b in t["blocks"]:
        if b["kind"] in ("heading", "paragraph"):
            yield b["text"]
        elif b["kind"] == "table":
            for row in b["rows"]:
                yield from row
        elif b["kind"] == "card":
            yield from b["texts"]


def score_text(t, text):
    have = set(tokens(text))
    ws = [w for x in truth_texts(t) for w in tokens(x) if len(w) >= 2]
    flat = normal(text)
    rows = [r for b in t["blocks"] if b["kind"] == "table" for r in b["rows"]]
    cards = [b["texts"] for b in t["blocks"] if b["kind"] == "card"]
    together = lambda parts: normal("".join(parts)) in flat  # noqa: E731
    return {
        "words_intact": sum(w in have for w in ws) / max(1, len(ws)),
        "rows_in_order": sum(together(r) for r in rows) / max(1, len(rows)),
        "cards_whole": sum(together(c) for c in cards) / max(1, len(cards)),
    }


def score_reflow(t, r):
    blocks = r["blocks"]
    # A picture's alt text is the table's text (read aloud, searched); the
    # cell separator is not in the truth.
    text = "\n".join(b["text"] for b in blocks).replace(" · ", " ")
    out = {"reflow_" + k: v for k, v in score_text(t, text).items()}
    rows = [r for b in t["blocks"] if b["kind"] == "table" and b["mode"] == "text" for r in b["rows"]]
    got = {normal(b["text"]) for b in blocks if b["kind"] == "paragraph"}
    out["text_rows"] = sum(normal(" · ".join(c for c in row if c)) in got for row in rows) / max(1, len(rows))
    pics = [b for b in t["blocks"] if b["kind"] == "table" and b["mode"] == "picture"]
    images = [b for b in blocks if b["kind"] == "image"]
    out["pictures"] = sum(any(normal(b["text"]).startswith(normal(p["rows"][0][0])) for b in images)
                          for p in pics) / max(1, len(pics))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engines", default="akkhor,mupdf,pdfium,poppler")
    ap.add_argument("--csv")
    ap.add_argument("--summary", help="averages per engine and producer, as CSV")
    args = ap.parse_args()
    engines = args.engines.split(",")
    rows = []
    for name in sorted(os.listdir(TABLES)):
        if not name.endswith(".pdf"):
            continue
        path = os.path.join(TABLES, name)
        t = json.load(open(path[:-4] + ".json", encoding="utf-8"))
        for e in engines:
            out, _ = call(e, "text", path)
            row = {"file": name, "engine": ENGINES[e]}
            row.update(score_text(t, "\n".join(out["pages"]) if out else ""))
            if e.startswith("akkhor"):
                r, _ = call(e, "reflow", path)
                if r and "blocks" in r:
                    row.update(score_reflow(t, r))
            rows.append(row)
            cells = " ".join(f"{k} {v:.3f}" for k, v in row.items() if k not in ("file", "engine"))
            print(f"{name:16} {row['engine']:8} {cells}")
    if args.csv:
        keys = sorted({k for r in rows for k in r}, key=lambda k: (k not in ("file", "engine"), k))
        with open(args.csv, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, keys)
            w.writeheader()
            w.writerows(rows)
    if args.summary:
        keys = sorted({k for r in rows for k in r if k not in ("file", "engine")})
        with open(args.summary, "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(["engine", "producer", "files"] + [k + "_pct" for k in keys])
            for e in engines:
                for producer in sorted({r["file"].split(".")[1] for r in rows}):
                    rs = [r for r in rows if r["engine"] == ENGINES[e] and r["file"].split(".")[1] == producer]
                    w.writerow([ENGINES[e], producer, len(rs)] + [
                        round(sum(r[k] for r in rs) / len(rs) * 100, 1) if all(k in r for r in rs) else ""
                        for k in keys])
    print()
    for e in engines:
        rs = [r for r in rows if r["engine"] == ENGINES[e]]
        keys = [k for k in rs[0] if k not in ("file", "engine")] if rs else []
        print(f"{ENGINES[e]:8} " + " ".join(
            f"{k} {sum(r[k] for r in rs if k in r) / max(1, len(rs)) * 100:.1f}%" for k in keys))


if __name__ == "__main__":
    main()
