#!/usr/bin/env python3
"""Draws the case-study figures from docs/case-study/data/ into
docs/case-study/figures/ (SVG for the web, PNG for posts).

    python3 scripts/bench/make_figures.py

Colour has one job here: Akkhor (blue) against the engines it is compared
with (orange); the engine names sit on the axis and every bar carries its
value, so nothing depends on colour alone. Text that needs Indic shaping
(the copied-text sample) is rendered by Chrome, which shapes Bangla and
Devanagari correctly; matplotlib does not.
"""

import csv
import json
import os
import shutil
import subprocess
import tempfile
from collections import defaultdict

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
DATA = os.path.join(ROOT, "docs", "case-study", "data")
FIG = os.path.join(ROOT, "docs", "case-study", "figures")

# Validated pair (dataviz validator: lightness, chroma, CVD and normal-vision
# separation pass; orange is under 3:1 on white, so every bar is labelled).
OURS, THEIRS = "#257de0", "#f47230"
TEXT, MUTED, GRID = "#191919", "#666666", "#e5e5e5"
ORDER = ["Akkhor", "Akkhor (+ device fonts)", "MuPDF", "PDFium", "Poppler"]

plt.rcParams.update({
    "font.family": "DejaVu Sans",
    "font.size": 10,
    "text.color": TEXT,
    "axes.labelcolor": MUTED,
    "axes.edgecolor": GRID,
    "xtick.color": MUTED,
    "ytick.color": TEXT,
    "axes.titleweight": "bold",
    "axes.titlesize": 11,
    "svg.fonttype": "none",
})


def read(name):
    path = os.path.join(DATA, name)
    return list(csv.DictReader(open(path, encoding="utf-8"))) if os.path.exists(path) else []


def env_note():
    e = json.load(open(os.path.join(DATA, "environment.json")))
    return (f"{e['cpu']}, {e['cores']} cores · MuPDF {e['mupdf']} · PDFium {e['pdfium']} · "
            f"{e['poppler'].replace('pdftoppm version', 'Poppler')} · Akkhor {e['akkhor_commit']} · {e['date']}")


def header(fig, title, subtitle=None):
    """Title and subtitle above the plots, placed in inches so they never
    run into the panel titles whatever the figure's height."""
    h = fig.get_figheight()
    lift = 0.55 if subtitle else 0.3
    fig.text(0.01, 1 + (lift + 0.1) / h, title, fontsize=13, fontweight="bold", ha="left", va="bottom")
    if subtitle:
        fig.text(0.01, 1 + 0.25 / h, subtitle, fontsize=8.5, color=MUTED, ha="left", va="bottom")


def save(fig, name, note=None):
    h = fig.get_figheight()
    fig.text(0.01, -0.85 / h, note or env_note(), fontsize=7, color=MUTED, ha="left", va="top")
    fig.savefig(os.path.join(FIG, name + ".svg"), bbox_inches="tight")
    fig.savefig(os.path.join(FIG, name + ".png"), dpi=200, bbox_inches="tight", facecolor="white")
    plt.close(fig)
    print("wrote", name)


def bars(ax, labels, values, fmt, title, xlabel, xmax=None):
    """Horizontal bars, one per engine, top to bottom in ORDER."""
    y = list(range(len(labels)))[::-1]
    colors = [OURS if l.startswith("Akkhor") else THEIRS for l in labels]
    ax.barh(y, values, height=0.55, color=colors, edgecolor="white", linewidth=2)
    ax.set_yticks(y, labels)
    ax.set_title(title, loc="left")
    ax.set_xlabel(xlabel)
    top = xmax or max(values + [1e-9]) * 1.25
    ax.set_xlim(0, top)
    for yy, v in zip(y, values):
        ax.text(v + top * 0.015, yy, fmt(v), va="center", fontsize=9, color=TEXT)
    ax.grid(axis="x", color=GRID, linewidth=0.8)
    ax.set_axisbelow(True)
    for s in ("top", "right", "left"):
        ax.spines[s].set_visible(False)
    ax.tick_params(axis="y", length=0)


def ordered(d):
    keys = [k for k in ORDER if k in d]
    return keys, [d[k] for k in keys]


# --- figures ---------------------------------------------------------------

def text_accuracy():
    rows = read("text_accuracy.csv")
    if not rows:
        return
    groups = {
        "PDFs as produced\n(Chrome, LibreOffice; 16 files)": lambda r: r["text_layer"] == "as produced" and "Bijoy" not in r["language"],
        "PDFs with no text layer\n(glyphs only; 16 files)": lambda r: r["text_layer"] == "no ToUnicode" and "Bijoy" not in r["language"],
        "Bijoy PDFs\n(SutonnyMJ; 4 files)": lambda r: "Bijoy" in r["language"],
    }
    fig, axes = plt.subplots(1, 3, figsize=(13, 3.6), sharey=True)
    for ax, (title, keep) in zip(axes, groups.items()):
        acc = defaultdict(list)
        for r in rows:
            if keep(r):
                acc[r["engine"]].append(min(1.0, float(r["cer"])) * 100)
        labels, vals = ordered({k: sum(v) / len(v) for k, v in acc.items()})
        bars(ax, labels, vals, lambda v: f"{v:.1f}%", title, "characters wrong (%, lower is better)", xmax=115)
    header(fig, "Bangla and Hindi text copied out of PDFs: how much of it is wrong", "Character error rate against the source text, whitespace ignored, capped at 100%. "
             "\"+ device fonts\": the PDF's font registered, as the Android app does with the phone's fonts.")
    save(fig, "text-accuracy")


def search():
    rows = read("text_accuracy.csv")
    if not rows:
        return
    acc = defaultdict(list)
    for r in rows:
        if r["text_layer"] == "as produced" and "Bijoy" not in r["language"]:
            acc[r["engine"]].append(float(r["words_found"]) * 100)
    labels, vals = ordered({k: sum(v) / len(v) for k, v in acc.items()})
    fig, ax = plt.subplots(figsize=(8, 3.2))
    bars(ax, labels, vals, lambda v: f"{v:.1f}%", "",
         "share of the source's words found (%, higher is better)", xmax=112)
    header(fig, "Words you can find by searching the copied text",
           "Same 16 PDFs as produced by Chrome and LibreOffice. A word is missed when an engine "
           "breaks it with stray spaces or joins it to its neighbour.")
    save(fig, "search")


def speed():
    rows = read("speed.csv")
    if not rows:
        return
    big = "big.pdf (2,000 pages, 308 MB)"
    pick = lambda metric, page, scale, f=big: {
        r["engine"]: float(r["best_ms"]) for r in rows
        if r["file"] == f and r["metric"] == metric and int(r["page"]) == page and float(r["scale"]) == scale}
    fig, axes = plt.subplots(1, 3, figsize=(13, 3.4), sharey=True)
    for ax, (title, d) in zip(axes, [
        ("Open (warm)", pick("open", 2000, 1.0)),
        ("Render the last page, first time (1x)", pick("first_render", 2000, 1.0)),
        ("Render the last page, first time (2x)", pick("first_render", 2000, 2.0)),
    ]):
        labels, vals = ordered(d)
        bars(ax, labels, vals, lambda v: f"{v:.2f} ms" if v < 10 else f"{v:.1f} ms", title, "milliseconds (best of 5, lower is better)")
    header(fig, "A 2,000-page, 308 MB PDF: open it and jump to the last page", "Libraries timed inside the process. Poppler runs as command-line tools; "
             "the start-up of `pdfinfo -v` is subtracted from its numbers.")
    save(fig, "speed-big-pdf")

    cases = [("indic.pdf (Bangla and Hindi page)", "Bangla and Hindi page (2x)"),
             ("bn.chrome.pdf (Bangla prose)", "Bangla prose page (2x)")]
    fig, axes = plt.subplots(1, 2, figsize=(10, 3.4), sharey=True)
    for ax, (f, title) in zip(axes, cases):
        labels, vals = ordered(pick("first_render", 1, 2.0, f))
        bars(ax, labels, vals, lambda v: f"{v:.1f} ms", title, "milliseconds (best of 5, lower is better)")
    header(fig, "Rendering an Indic page")
    save(fig, "speed-indic-pages")

    # Where Akkhor is behind: drawing the same page again.
    fig, axes = plt.subplots(1, 2, figsize=(10, 3.4), sharey=True)
    for ax, (f, title) in zip(axes, cases):
        labels, vals = ordered(pick("warm_render", 1, 2.0, f))
        bars(ax, labels, vals, lambda v: f"{v:.1f} ms", title, "milliseconds (best of 5, lower is better)")
    header(fig, "Drawing the same page a second time",
           "Every engine keeps what it parsed between draws. Akkhor shares one render cache per document "
           "since Phase 3.3 (5.4 → 3.0 ms on the first page); MuPDF stays ahead.")
    save(fig, "speed-redraw")


def cold_open():
    rows = read("cold_open.csv")
    if not rows:
        return
    fig, axes = plt.subplots(1, 2, figsize=(11, 2.9), sharey=True)
    for ax, (col, title) in zip(axes, [("cold_open_ms", "Open"),
                                       ("cold_open_plus_last_page_ms", "Open and draw the last page (1x)")]):
        labels, vals = ordered({r["engine"]: float(r[col]) for r in rows if r.get(col)})
        bars(ax, labels, vals, lambda v: f"{v:.1f} ms", title, "milliseconds (median of 3, lower is better)")
    header(fig, "Opening the 2,000-page PDF straight from disk (cold cache)",
           "OS file cache dropped before every run. Poppler is left out: as a new process, "
           "its own start-up is also cold and would dominate.")
    save(fig, "cold-open")


def memory():
    rows = read("memory.csv")
    if not rows:
        return
    cases = sorted({r["case"] for r in rows})
    fig, axes = plt.subplots(1, len(cases), figsize=(5 * len(cases), 3.2), sharey=True)
    for ax, case in zip(axes, cases):
        labels, vals = ordered({r["engine"]: float(r["added_mb"]) for r in rows if r["case"] == case})
        bars(ax, labels, vals, lambda v: f"{v:.1f} MB", case, "memory added (MB, lower is better)")
    header(fig, "Memory to open a PDF and render one page", "Resident memory added between just before opening and the peak after rendering; "
             "median of 3 fresh processes.")
    save(fig, "memory")


def size():
    rows = read("size.csv")
    if not rows:
        return
    labels, vals = ordered({r["engine"]: int(r["bytes"]) / 1e6 for r in rows})
    fig, ax = plt.subplots(figsize=(8, 2.9))
    bars(ax, labels, vals, lambda v: f"{v:.1f} MB", "", "megabytes")
    scope = {r["engine"]: r["scope"] for r in rows}
    header(fig, "Library size (x86_64 Linux, stripped)",
           "Not like for like. Akkhor: " + scope.get("Akkhor", "") + ". MuPDF: " + scope.get("MuPDF", "")
           + ".\nPDFium: PDF only. Poppler: PDF only; FreeType, cairo and fontconfig not counted.")
    save(fig, "size")


def robustness():
    rows = read("robustness.csv")
    if not rows:
        return
    by = {r["engine"]: r for r in rows}
    labels = [k for k in ORDER if k in by]
    opened = [int(by[k]["opened"]) for k in labels]
    fig, ax = plt.subplots(figsize=(8, 2.9))
    total = int(rows[0]["files"])
    bars(ax, labels, opened, lambda v: "", "", "files opened and rendered (higher is better)", xmax=total * 1.55)
    y = list(range(len(labels)))[::-1]
    for yy, k, o in zip(y, labels, opened):
        r = by[k]
        bad = int(r["crashed"]) + int(r["hung"])
        ax.text(o + total * 0.02, yy, f"{o} opened · {r['refused']} refused cleanly · {bad} crashed or hung",
                va="center", fontsize=9)
    header(fig, f"{total} damaged PDFs: does the engine survive them?",
           "Bytes flipped, files cut short, blocks duplicated or zeroed; the same files for every engine. "
           "Refusing with an error is fine; crashing or hanging is not.")
    save(fig, "robustness")


def progress():
    rows = read("history.csv")
    pairs = [("warm open 2000-page 308 MB PDF", "Open, warm (ms)"),
             ("heap after open 2000-page PDF", "Heap after open (MB)"),
             ("file touched at open 2000-page PDF", "File read into memory (MB)")]
    fig, axes = plt.subplots(1, 3, figsize=(12, 2.6))
    for ax, (metric, title) in zip(axes, pairs):
        vals = [float(r["value"]) for r in rows if r["metric"] == metric]
        if len(vals) < 2:
            continue
        # Both bars are Akkhor: a lighter step of its blue for "before".
        ax.barh([1, 0], vals[:2], height=0.55, color=["#84b9f3", OURS], edgecolor="white", linewidth=2)
        ax.set_yticks([1, 0], ["before", "after"])
        ax.set_title(title, loc="left")
        top = max(vals[:2]) * 1.35
        ax.set_xlim(0, top)
        for yy, v in zip([1, 0], vals[:2]):
            ax.text(v + top * 0.02, yy, f"{v:g}", va="center", fontsize=9)
        for s in ("top", "right", "left"):
            ax.spines[s].set_visible(False)
        ax.tick_params(axis="y", length=0)
    header(fig, "Phase 3.2: reading the page tree lazily (2,000-page PDF)")
    save(fig, "progress-lazy-page-tree", note="Desktop, warm file cache. Source: docs/case-study/data/history.csv")

    # Phase 3 as a whole: the first and last value of each metric, except the
    # library size, compared with the build just before Phase 3.6.
    def first_last(metric, before_where=None):
        r = [x for x in rows if x["metric"] == metric]
        if before_where:
            r = [x for x in r if before_where in x["source"]] + [r[-1]]
        return [float(r[0]["value"]), float(r[-1]["value"])] if len(r) >= 2 else None

    panels = [("Cold open, 2,000-page PDF (ms, CI)", first_last("cold open 2000-page 308 MB PDF")),
              ("Redraw a Bangla/Hindi page at 2x (ms)", first_last("redraw Bangla and Hindi page at 2x")),
              ("Heap after reading 200 pages (MB)", first_last("heap after 200 pages read")),
              ("Android arm64 library (MB)", first_last("arm64 Android library size", "before 3.6"))]
    panels = [p for p in panels if p[1]]
    fig, axes = plt.subplots(1, len(panels), figsize=(4 * len(panels), 2.6))
    for ax, (title, vals) in zip(axes, panels):
        ax.barh([1, 0], vals, height=0.55, color=["#84b9f3", OURS], edgecolor="white", linewidth=2)
        ax.set_yticks([1, 0], ["before", "after"])
        ax.set_title(title, loc="left", fontsize=10)
        top = max(vals) * 1.35
        ax.set_xlim(0, top)
        for yy, v in zip([1, 0], vals):
            ax.text(v + top * 0.02, yy, f"{v:g}", va="center", fontsize=9)
        for sp in ("top", "right", "left"):
            ax.spines[sp].set_visible(False)
        ax.tick_params(axis="y", length=0)
    header(fig, "Phase 3: speed, memory and size, before and after")
    save(fig, "progress-phase-3", note="Each value and where it was measured: docs/case-study/data/history.csv")


# --- pictures made by the engine and by Chrome -----------------------------

def chrome_png(html, out, width=1400):
    with tempfile.TemporaryDirectory() as d:
        page = os.path.join(d, "page.html")
        open(page, "w", encoding="utf-8").write(html)
        subprocess.run(["google-chrome", "--headless", "--no-sandbox", "--disable-gpu", "--hide-scrollbars",
                        f"--window-size={width},2000", f"--screenshot={out}", "file://" + page],
                       capture_output=True, timeout=120)
    # Trim the empty bottom.
    from PIL import Image, ImageChops
    im = Image.open(out).convert("RGB")
    bbox = ImageChops.difference(im, Image.new("RGB", im.size, "white")).getbbox()
    if bbox:
        im.crop((0, 0, im.width, min(im.height, bbox[3] + 24))).save(out)


def samples():
    path = os.path.join(DATA, "samples.json")
    if not os.path.exists(path):
        return
    data = json.load(open(path, encoding="utf-8"))
    import html as h
    shown = [("pdf/bn.chrome.pdf", "Bangla, printed by Chrome"), ("pdf/hi.chrome.pdf", "Hindi, printed by Chrome"),
             ("pdf/bn.bijoy.chrome.pdf", "Bangla typed in Bijoy (SutonnyMJ)")]
    blocks = []
    for f, title in shown:
        v = data.get(f)
        if not v:
            continue
        truth = v["truth"]
        rows = []
        for e in ["Akkhor", "MuPDF", "PDFium", "Poppler"]:
            got = v.get(e, "")
            n = min(len(got), 64)
            ok = got.split() == truth.split()[:len(got.split())]
            rows.append(f"<tr><th class='{'ours' if e == 'Akkhor' else ''}'>{e}</th>"
                        f"<td>{h.escape(got[:n])}{'…' if len(got) > n else ''}</td>"
                        f"<td class='mark'>{'✓' if ok else '✗'}</td></tr>")
        blocks.append(f"<h2>{h.escape(title)}</h2><table><tr><th class='src'>In the PDF</th>"
                      f"<td class='src'>{h.escape(truth[:64])}…</td><td></td></tr>{''.join(rows)}</table>")
    html = f"""<!doctype html><meta charset="utf-8"><style>
    body {{ font-family: 'Noto Serif Bengali','Noto Serif Devanagari','Noto Serif',serif; margin: 28px 32px;
            color: {TEXT}; background: white; width: 1300px; }}
    h1 {{ font: 700 22px 'DejaVu Sans', sans-serif; margin: 0 0 4px; }}
    p.sub {{ font: 13px 'DejaVu Sans', sans-serif; color: {MUTED}; margin: 0 0 18px; }}
    h2 {{ font: 700 14px 'DejaVu Sans', sans-serif; margin: 18px 0 6px; color: {MUTED}; }}
    table {{ border-collapse: collapse; width: 100%; }}
    th {{ font: 600 13px 'DejaVu Sans', sans-serif; text-align: left; width: 120px; padding: 6px 10px 6px 0;
          color: {TEXT}; vertical-align: top; }}
    th.ours {{ color: {OURS}; }}
    td {{ font-size: 19px; padding: 5px 0; border-top: 1px solid {GRID}; }}
    td.src, th.src {{ color: {MUTED}; }}
    td.mark {{ font: 16px 'DejaVu Sans', sans-serif; width: 28px; text-align: right; }}
    </style><h1>What you get when you copy text out of the PDF</h1>
    <p class="sub">First line of each corpus PDF as extracted by each engine. ✓ = every word intact.
    Data: docs/case-study/data/samples.json</p>{''.join(blocks)}"""
    out = os.path.join(FIG, "copied-text-samples.png")
    chrome_png(html, out)
    print("wrote copied-text-samples")


def themes():
    cli = os.path.join(ROOT, "target", "release", "akkhor")
    pdf = os.path.join(ROOT, "tests", "fixtures", "indic.pdf")
    if not os.path.exists(cli):
        return
    from PIL import Image, ImageDraw, ImageFont
    names = ["original", "paper", "sepia", "dusk", "night"]
    tiles = []
    with tempfile.TemporaryDirectory() as d:
        for t in names:
            out = os.path.join(d, t + ".png")
            subprocess.run([cli, pdf, "--theme", t, "--scale", "0.9", "--out", out], capture_output=True)
            im = Image.open(out).convert("RGB")
            tiles.append(im)
    # Cut every page at the same blank row near the middle (the original's
    # rows that hold no ink), so no line of text is cut in half.
    orig = tiles[0]
    px = orig.load()
    cut = int(orig.height * 0.55)
    while cut > orig.height // 3 and any(px[x, cut] != px[0, 0] for x in range(orig.width)):
        cut -= 1
    tiles = [im.crop((0, 0, im.width, cut + 8)) for im in tiles]
    w, h = tiles[0].size
    pad, head = 16, 34
    sheet = Image.new("RGB", (len(tiles) * (w + pad) + pad, h + head + pad), "white")
    draw = ImageDraw.Draw(sheet)
    try:
        font = ImageFont.truetype("DejaVuSans-Bold.ttf", 18)
    except OSError:
        font = ImageFont.load_default()
    for i, (t, im) in enumerate(zip(names, tiles)):
        x = pad + i * (w + pad)
        sheet.paste(im, (x, head))
        draw.rectangle([x - 1, head - 1, x + w, head + h], outline=GRID)
        draw.text((x, 8), t.capitalize(), fill=TEXT, font=font)
    sheet.save(os.path.join(FIG, "themes.png"))
    print("wrote themes")


def main():
    os.makedirs(FIG, exist_ok=True)
    for f in (text_accuracy, search, speed, cold_open, memory, size, robustness, progress, samples, themes):
        f()
    if shutil.which("google-chrome") is None:
        print("note: google-chrome not found; the text sample needs it")


if __name__ == "__main__":
    main()
