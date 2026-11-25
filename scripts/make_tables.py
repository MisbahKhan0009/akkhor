#!/usr/bin/env python3
"""Builds the table and card corpus in tests/corpus/tables/pdf from
tests/corpus/tables/*-tables.txt, with a ground-truth JSON beside each PDF.

Source format (one block per line):
  `# ` / `## `            heading, level 1 or 2
  `! text`                a bold lead-in line (a paragraph, not a heading)
  `| a | b |`             a table row; consecutive rows make a table, the
                          first is the header
  `@ title :: p1 :: p2`   a full-width card: bold title, centred paragraphs
  `@@ t :: b || t :: b`   cards side by side, each a title and a body
  anything else           a paragraph

Every producer sets the three tables differently, so each kind of table
appears somewhere: fully ruled grids, horizontal rules only (booktabs, a
Typst row stroke) and no rules at all (Chrome, Typst).

Truth: blocks in reading order. A table carries its rows and how a reflow
should show it: as text when it has at most three columns, as a picture
when wider. A card carries its texts in order. Beside each JSON, a
`.expect` file spells the same truth as the reflow should give it, one
block per line, for the Rust tests.

Needs google-chrome, soffice, xelatex (tcolorbox, booktabs) and the typst
and python-docx Python packages. Run from the repository root:
    python3 scripts/make_tables.py
"""

import html
import json
import os
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from make_books import FONT_DIRS, FONTS, LANG, font_path, tex_escape  # noqa: E402

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
SRC = os.path.join(ROOT, "tests", "corpus", "tables")
OUT = os.path.join(SRC, "pdf")
HEADER = {"bn": "শব্দভান্ডার · অক্ষর পরীক্ষা", "hi": "शब्दावली · अक्षर परीक्षण"}
TEXT_COLUMNS = 3  # tables up to this wide reflow as text, wider as a picture
CARD_FILL = "#E6EDF7"


def parse(path):
    blocks = []
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if not line:
            continue
        if line.startswith("## "):
            blocks.append(("heading", 2, line[3:]))
        elif line.startswith("# "):
            blocks.append(("heading", 1, line[2:]))
        elif line.startswith("! "):
            blocks.append(("leadin", 0, line[2:]))
        elif line.startswith("|"):
            row = [c.strip() for c in line.strip("|").split("|")]
            if blocks and blocks[-1][0] == "table":
                blocks[-1][2].append(row)
            else:
                blocks.append(("table", 0, [row]))
        elif line.startswith("@@ "):
            cards = [[p.strip() for p in c.split("::")] for c in line[3:].split("||")]
            blocks.append(("cards", 0, cards))
        elif line.startswith("@ "):
            blocks.append(("card", 0, [p.strip() for p in line[2:].split("::")]))
        else:
            blocks.append(("paragraph", 0, line))
    return blocks


def truth(blocks, lang):
    out = []
    for kind, level, data in blocks:
        if kind == "heading":
            out.append({"kind": "heading", "level": level, "text": data})
        elif kind == "paragraph":
            out.append({"kind": "paragraph", "text": data})
        elif kind == "leadin":
            out.append({"kind": "paragraph", "text": data, "bold": True})
        elif kind == "table":
            mode = "text" if len(data[0]) <= TEXT_COLUMNS else "picture"
            out.append({"kind": "table", "mode": mode, "rows": data})
        elif kind == "card":
            out.append({"kind": "card", "texts": data})
        elif kind == "cards":
            for c in data:
                out.append({"kind": "card", "texts": c})
    return {"lang": lang, "blocks": out, "furniture": [HEADER[lang]]}


# --- producers -----------------------------------------------------------------

def chrome(blocks, lang, family, pdf):
    body, tables = [], 0
    for kind, level, data in blocks:
        if kind == "heading":
            body.append(f"<h{level}>{html.escape(data)}</h{level}>")
        elif kind == "paragraph":
            body.append(f"<p>{html.escape(data)}</p>")
        elif kind == "leadin":
            body.append(f"<p><b>{html.escape(data)}</b></p>")
        elif kind == "table":
            # ruled, then no rules at all, then ruled
            cls = "plain" if tables == 1 else "ruled"
            tables += 1
            rows = "".join(
                "<tr>" + "".join(f"<{'th' if i == 0 else 'td'}>{html.escape(c)}</{'th' if i == 0 else 'td'}>"
                                 for c in row) + "</tr>"
                for i, row in enumerate(data))
            body.append(f'<table class="{cls}">{rows}</table>')
        elif kind in ("card", "cards"):
            cards = [data] if kind == "card" else data
            inner = "".join(
                '<div class="card"><p class="t">' + html.escape(c[0]) + "</p>"
                + "".join(f"<p>{html.escape(x)}</p>" for x in c[1:]) + "</div>"
                for c in cards)
            body.append(f'<div class="{"cards" if kind == "cards" else "one"}">{inner}</div>')
    counter = "counter(page, bengali)" if lang == "bn" else "counter(page)"
    doc = f"""<!doctype html><html lang="{LANG[lang][1]}"><meta charset="utf-8"><style>
@page {{ size: A5; margin: 20mm 14mm;
  @top-center {{ content: "{HEADER[lang]}"; font-family: '{family}'; font-size: 8pt; }}
  @bottom-center {{ content: {counter}; font-family: '{family}'; font-size: 9pt; }} }}
body {{ font-family: '{family}', serif; font-size: 11pt; line-height: 1.5; margin: 0; }}
h1 {{ font-size: 18pt; margin: 0 0 8pt; }} h2 {{ font-size: 13.5pt; margin: 14pt 0 6pt; }}
p {{ margin: 0 0 0.6em; }}
table {{ border-collapse: collapse; width: 100%; margin: 4pt 0 10pt; break-inside: avoid; }}
th, td {{ text-align: left; vertical-align: top; padding: 3pt 6pt; }}
table.ruled th, table.ruled td {{ border: 0.75pt solid #444; }}
table.ruled th {{ background: #eee; }}
.card {{ background: {CARD_FILL}; border-radius: 6pt; padding: 8pt 10pt; text-align: center; break-inside: avoid; }}
.card p {{ margin: 0 0 4pt; }} .card p.t {{ font-weight: bold; }}
.one {{ margin: 8pt 0 10pt; }}
.cards {{ display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 6mm; margin: 8pt 0 10pt; }}
</style><body>{"".join(body)}</body></html>"""
    with tempfile.NamedTemporaryFile("w", suffix=".html", delete=False, encoding="utf-8") as f:
        f.write(doc)
    subprocess.run(["google-chrome", "--headless", "--no-sandbox", "--disable-gpu", "--no-pdf-header-footer",
                    f"--print-to-pdf={pdf}", "file://" + f.name], check=True, capture_output=True, timeout=120)


def writer(blocks, lang, family, pdf):
    """A Word document (.docx) printed by LibreOffice Writer: tables in the
    "Table Grid" style (ruled), cards as shaded table cells, the way Word
    users make boxes."""
    from docx import Document
    from docx.enum.table import WD_TABLE_ALIGNMENT
    from docx.enum.text import WD_ALIGN_PARAGRAPH
    from docx.oxml import OxmlElement
    from docx.oxml.ns import qn
    from docx.shared import Mm, Pt

    d = Document()
    sec = d.sections[0]
    sec.page_width, sec.page_height = Mm(148), Mm(210)
    sec.left_margin = sec.right_margin = Mm(14)
    sec.top_margin = sec.bottom_margin = Mm(20)

    def font(run, size, bold=False):
        run.font.name = family
        run.font.size = Pt(size)
        run.bold = bold
        rpr = run._element.get_or_add_rPr()
        fonts = rpr.find(qn("w:rFonts"))
        for a in ("w:ascii", "w:hAnsi", "w:cs", "w:eastAsia"):
            fonts.set(qn(a), family)
        cs = OxmlElement("w:szCs")
        cs.set(qn("w:val"), str(int(size * 2)))
        rpr.append(cs)
        if bold:
            b = OxmlElement("w:bCs")
            rpr.append(b)

    def para(container, text, size=11, bold=False, align=None, after=6):
        p = container.add_paragraph() if hasattr(container, "add_paragraph") else container
        font(p.add_run(text), size, bold)
        p.paragraph_format.space_after = Pt(after)
        if align is not None:
            p.alignment = align
        return p

    def shade(cell):
        tcpr = cell._element.get_or_add_tcPr()
        shd = OxmlElement("w:shd")
        shd.set(qn("w:val"), "clear")
        shd.set(qn("w:color"), "auto")
        shd.set(qn("w:fill"), CARD_FILL[1:])
        tcpr.append(shd)

    def keep_rows(table):
        # Rows are not split over pages (Word's "allow row to break" off).
        for row in table.rows:
            trpr = row._tr.get_or_add_trPr()
            trpr.append(OxmlElement("w:cantSplit"))

    def no_borders(table):
        tblpr = table._element.tblPr
        borders = OxmlElement("w:tblBorders")
        for edge in ("top", "left", "bottom", "right", "insideH", "insideV"):
            e = OxmlElement(f"w:{edge}")
            e.set(qn("w:val"), "nil")
            borders.append(e)
        tblpr.append(borders)

    hp = sec.header.paragraphs[0]
    hp.alignment = WD_ALIGN_PARAGRAPH.CENTER
    font(hp.add_run(HEADER[lang]), 8)
    fp = sec.footer.paragraphs[0]
    fp.alignment = WD_ALIGN_PARAGRAPH.CENTER
    run = fp.add_run()
    font(run, 9)
    for tag, text in (("begin", None), (None, "PAGE"), ("end", None)):
        if tag:
            el = OxmlElement("w:fldChar")
            el.set(qn("w:fldCharType"), tag)
        else:
            el = OxmlElement("w:instrText")
            el.text = text
        run._element.append(el)

    for kind, level, data in blocks:
        if kind == "heading":
            para(d, data, {1: 18, 2: 13.5}[level], bold=True)
        elif kind == "paragraph":
            para(d, data)
        elif kind == "leadin":
            para(d, data, bold=True)
        elif kind == "table":
            t = d.add_table(rows=len(data), cols=len(data[0]))
            t.style = "Table Grid"
            t.alignment = WD_TABLE_ALIGNMENT.CENTER
            keep_rows(t)
            for i, row in enumerate(data):
                for j, c in enumerate(row):
                    para(t.cell(i, j).paragraphs[0], c, 10.5, bold=i == 0, after=0)
            para(d, "", 6, after=0)
        elif kind in ("card", "cards"):
            cards = [data] if kind == "card" else data
            t = d.add_table(rows=1, cols=len(cards))
            no_borders(t)
            keep_rows(t)
            for j, c in enumerate(cards):
                cell = t.cell(0, j)
                shade(cell)
                para(cell.paragraphs[0], c[0], 11, bold=True, align=WD_ALIGN_PARAGRAPH.CENTER, after=3)
                for x in c[1:]:
                    para(cell, x, 11, align=WD_ALIGN_PARAGRAPH.CENTER, after=3)
            para(d, "", 6, after=0)
    work = tempfile.mkdtemp()
    src = os.path.join(work, os.path.basename(pdf)[:-4] + ".docx")
    d.save(src)
    subprocess.run(["soffice", "--headless", "--convert-to", "pdf", "--outdir", work, src],
                   check=True, capture_output=True, timeout=180)
    shutil.move(src[:-5] + ".pdf", pdf)


def xelatex(blocks, lang, family, pdf):
    """Tables: a ruled grid (|l|l| and \\hline), then booktabs (horizontal
    rules only) twice; cards in tcolorbox, side by side in a raster."""
    fname = dict(FONTS[lang])[family]
    fdir = os.path.dirname(font_path(fname)) + "/"
    script = "Bengali" if lang == "bn" else "Devanagari"
    fill = CARD_FILL[1:]
    lines = [
        r"\documentclass[11pt]{article}",
        r"\usepackage[a5paper,margin=16mm,headsep=6mm]{geometry}",
        r"\usepackage{fontspec}\usepackage{booktabs}\usepackage{array}",
        r"\usepackage[most]{tcolorbox}\tcbuselibrary{raster}",
        rf"\definecolor{{cardfill}}{{HTML}}{{{fill}}}",
        rf"\setmainfont{{{fname}}}[Path={fdir},Script={script}]",
        rf"\usepackage{{polyglossia}}\setdefaultlanguage{{{LANG[lang][0]}}}",
        rf"\newfontfamily\{LANG[lang][0]}font{{{fname}}}[Path={fdir},Script={script}]",
        r"\usepackage{fancyhdr}\pagestyle{fancy}\fancyhf{}\renewcommand{\headrulewidth}{0pt}",
        rf"\fancyhead[C]{{\footnotesize {HEADER[lang]}}}\fancyfoot[C]{{\thepage}}",
        r"\setlength{\parindent}{0pt}\setlength{\parskip}{6pt}",
        r"\tcbset{colback=cardfill,colframe=cardfill,arc=2mm,halign=center,boxrule=0pt}",
        r"\begin{document}\raggedright",
    ]
    tables = 0
    for kind, level, data in blocks:
        if kind == "heading":
            lines.append((r"\section*{" if level == 1 else r"\subsection*{") + tex_escape(data) + "}")
        elif kind == "paragraph":
            lines.append(tex_escape(data) + "\n")
        elif kind == "leadin":
            lines.append(r"\textbf{" + tex_escape(data) + "}\n")
        elif kind == "table":
            n = len(data[0])
            head = " & ".join(r"\textbf{" + tex_escape(c) + "}" for c in data[0]) + r" \\"
            body = [" & ".join(tex_escape(c) for c in row) + r" \\" for row in data[1:]]
            if tables == 0:
                spec = "|" + "|".join(["l"] * n) + "|"
                rows = [r"\hline", head, r"\hline"] + [b + r" \hline" for b in body]
            else:
                spec = "@{}p{0.17\\linewidth}p{0.3\\linewidth}p{0.43\\linewidth}@{}" if n == 3 \
                    else "@{}l" + "r" * (n - 1) + "@{}"
                rows = [r"\toprule", head, r"\midrule"] + body + [r"\bottomrule"]
            tables += 1
            lines.append(r"\begin{center}\begin{tabular}{" + spec + "}\n" + "\n".join(rows)
                         + "\n\\end{tabular}\\end{center}\n")
        elif kind in ("card", "cards"):
            cards = [data] if kind == "card" else data

            def box(c):
                return (r"\begin{tcolorbox}\textbf{" + tex_escape(c[0]) + r"}\par "
                        + r"\par ".join(tex_escape(x) for x in c[1:]) + r"\end{tcolorbox}")
            if kind == "card":
                lines.append(box(cards[0]) + "\n")
            else:
                lines.append(rf"\begin{{tcbraster}}[raster columns={len(cards)},raster equal height]"
                             + "".join(box(c) for c in cards) + r"\end{tcbraster}" + "\n")
    lines.append(r"\end{document}")
    work = tempfile.mkdtemp()
    tex = os.path.join(work, "t.tex")
    open(tex, "w", encoding="utf-8").write("\n".join(lines))
    r = subprocess.run(["xelatex", "-interaction=nonstopmode", "-halt-on-error", "t.tex"], cwd=work,
                       capture_output=True, timeout=300)
    if not os.path.exists(os.path.join(work, "t.pdf")):
        raise RuntimeError("xelatex failed:\n" + r.stdout.decode(errors="replace")[-1500:])
    shutil.move(os.path.join(work, "t.pdf"), pdf)


def typst_doc(blocks, lang, family, pdf):
    """Tables: Typst's default grid (ruled), no strokes, then row strokes
    only; cards as filled blocks, side by side in a grid."""
    import typst

    def q(s):
        return s.replace("\\", "\\\\").replace("#", "\\#").replace("*", "\\*").replace("_", "\\_") \
            .replace("[", "\\[").replace("]", "\\]").replace("$", "\\$").replace("@", "\\@").replace("<", "\\<")

    src = [
        '#set page(width: 148mm, height: 210mm, margin: (x: 14mm, y: 20mm),',
        f'  header: align(center, text(size: 8pt)[{q(HEADER[lang])}]),',
        '  footer: context align(center, text(size: 9pt, counter(page).display())))',
        f'#set text(font: "{family}", size: 11pt, lang: "{LANG[lang][1]}")',
        '#set par(spacing: 1em)',
        '#show heading.where(level: 1): set text(size: 18pt)',
        '#show heading.where(level: 2): set text(size: 13.5pt)',
        f'#let card(body) = block(fill: rgb("{CARD_FILL}"), inset: 8pt, radius: 4pt, width: 100%,'
        ' align(center, body))',
    ]
    tables = 0
    for kind, level, data in blocks:
        if kind == "heading":
            src.append("=" * level + " " + q(data))
        elif kind == "paragraph":
            src.append(q(data) + "\n")
        elif kind == "leadin":
            src.append("*" + q(data) + "*\n")
        elif kind == "table":
            n = len(data[0])
            stroke = ["", "stroke: none, ", "stroke: (x: none, y: 0.5pt), "][tables % 3]
            tables += 1
            cols = "(auto, auto, 1fr)" if n == 3 else str(n)
            cells = ", ".join("[*" + q(c) + "*]" for c in data[0])
            cells += ", " + ", ".join("[" + q(c) + "]" for row in data[1:] for c in row)
            src.append(f"#table(columns: {cols}, {stroke}inset: 5pt, {cells})\n")
        elif kind in ("card", "cards"):
            cards = [data] if kind == "card" else data

            def box(c):
                return "card[*" + q(c[0]) + "*\n\n" + "\n\n".join(q(x) for x in c[1:]) + "]"
            if kind == "card":
                src.append("#" + box(cards[0]) + "\n")
            else:
                src.append(f"#grid(columns: {len(cards)}, gutter: 6pt, "
                           + ", ".join(box(c) for c in cards) + ")\n")
    work = tempfile.mkdtemp()
    path = os.path.join(work, "t.typ")
    open(path, "w", encoding="utf-8").write("\n".join(src))
    typst.compile(path, output=pdf, font_paths=FONT_DIRS, root=work)


def expected(t):
    """The truth as the reflow should give it, one block per line
    (`kind<TAB>text`), for the Rust tests, which read no JSON: heading N,
    paragraph, row (a table row as text, cells joined with " · "), picture
    (a table as a picture: its first cell) and card (a card's texts)."""
    out = []
    for b in t["blocks"]:
        if b["kind"] == "heading":
            out.append(f"heading {b['level']}\t{b['text']}")
        elif b["kind"] == "paragraph":
            out.append(f"paragraph\t{b['text']}")
        elif b["kind"] == "table" and b["mode"] == "text":
            out += [f"row\t{' · '.join(c for c in row if c)}" for row in b["rows"]]
        elif b["kind"] == "table":
            out.append(f"picture\t{b['rows'][0][0]}")
        elif b["kind"] == "card":
            # A card is scored whole: its texts together and in order. How
            # a card splits into paragraphs inside is often not visible (no
            # bold font, no paragraph space), so it is not asked for.
            out.append("card\t" + " ".join(b["texts"]))
    return "\n".join(out) + "\n"


PRODUCERS = [("chrome", chrome), ("writer", writer), ("xelatex", xelatex), ("typst", typst_doc)]


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else None
    if only == "expect":  # rewrite the .expect files from the JSON truth
        for name in sorted(os.listdir(OUT)):
            if name.endswith(".json"):
                t = json.load(open(os.path.join(OUT, name), encoding="utf-8"))
                open(os.path.join(OUT, name[:-5] + ".expect"), "w", encoding="utf-8").write(expected(t))
        return
    os.makedirs(OUT, exist_ok=True)
    for lang in ("bn", "hi"):
        blocks = parse(os.path.join(SRC, f"{lang}-tables.txt"))
        family = FONTS[lang][0][0]
        for producer, make in PRODUCERS:
            if only and only != producer:
                continue
            stem = f"{lang}.{producer}"
            pdf = os.path.join(OUT, stem + ".pdf")
            try:
                make(blocks, lang, family, pdf)
            except Exception as e:  # a missing producer must not stop the rest
                print(f"{stem}: FAILED {e}")
                continue
            t = truth(blocks, lang)
            json.dump(t, open(os.path.join(OUT, stem + ".json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
            open(os.path.join(OUT, stem + ".expect"), "w", encoding="utf-8").write(expected(t))
            print(f"{stem}.pdf  {os.path.getsize(pdf)} bytes")


if __name__ == "__main__":
    main()
