#!/usr/bin/env python3
"""Builds the book-layout corpus in tests/corpus/books/pdf from the sources
in tests/corpus/books/*-book.txt, with a ground-truth JSON beside each PDF.

Source format: `# ` heading level 1, `## ` level 2, `^ ` a footnote to the
paragraph before it, any other line a paragraph.

Producers: Chrome, LibreOffice Writer (from a .docx, the path Word
documents take), XeLaTeX, LuaLaTeX (HarfBuzz renderer) and Typst.
Layouts:
  one  one column, ragged right, running header and page number
  two  two columns, justified and hyphenated, running header, page number,
       a footnote (where the producer has footnotes) and a figure
Each PDF's truth lists its blocks in reading order (heading with level,
paragraph, footnote, image) and the furniture (running header and page
numbers) that must not reach a reflow.

Needs google-chrome, soffice, xelatex, lualatex and the typst, python-docx,
Pillow Python packages. Run from the repository root:
    python3 scripts/make_books.py
"""

import html
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
SRC = os.path.join(ROOT, "tests", "corpus", "books")
OUT = os.path.join(SRC, "pdf")
FONT_DIRS = [os.path.join(ROOT, "fonts"), os.path.join(ROOT, "tests", "fonts")]

# family name, file, and the second design used for the two-column layout
FONTS = {
    "bn": [("Noto Serif Bengali", "NotoSerifBengali-Regular.ttf"), ("Hind Siliguri", "HindSiliguri-Regular.ttf"),
           ("Kalpurush", "Kalpurush-Regular.ttf")],
    "hi": [("Noto Serif Devanagari", "NotoSerifDevanagari-Regular.ttf"), ("Mukta", "Mukta-Regular.ttf")],
}
HEADER = {"bn": "নদীর ধারে একটি গ্রাম · অক্ষর পরীক্ষা", "hi": "पहाड़ों के बीच एक कस्बा · अक्षर परीक्षण"}
LANG = {"bn": ("bengali", "bn"), "hi": ("hindi", "hi")}
FIGURE_AFTER = 5  # the figure follows the fifth paragraph


def font_path(name):
    for d in FONT_DIRS:
        p = os.path.join(d, name)
        if os.path.exists(p):
            return p
    raise FileNotFoundError(name)


def parse(path):
    """Blocks of a source file: (kind, level, text)."""
    blocks = []
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if not line:
            continue
        if line.startswith("## "):
            blocks.append(("heading", 2, line[3:]))
        elif line.startswith("# "):
            blocks.append(("heading", 1, line[2:]))
        elif line.startswith("^ "):
            blocks.append(("footnote", 0, line[2:]))
        else:
            blocks.append(("paragraph", 0, line))
    return blocks


def with_figure(blocks):
    """Inserts the figure after the FIGURE_AFTER-th paragraph."""
    out, n = [], 0
    for b in blocks:
        out.append(b)
        if b[0] == "paragraph":
            n += 1
            if n == FIGURE_AFTER:
                out.append(("image", 0, ""))
    return out


def make_figure(path):
    """A colourful 600x360 picture (a photo stand-in), so themes and image
    blocks have something real to keep."""
    from PIL import Image, ImageDraw

    img = Image.new("RGB", (600, 360))
    d = ImageDraw.Draw(img)
    for y in range(360):
        d.line([(0, y), (600, y)], fill=(30 + y // 3, 90 + y // 4, 160 - y // 4))
    d.ellipse([400, 40, 520, 160], fill=(250, 200, 60))
    d.polygon([(0, 360), (180, 150), (360, 360)], fill=(40, 110, 60))
    d.polygon([(220, 360), (420, 120), (600, 360)], fill=(30, 90, 50))
    img.save(path)


def truth(blocks, lang, layout, footnotes):
    keep = []
    for kind, level, text in blocks:
        if kind == "footnote" and not footnotes:
            continue
        keep.append({"kind": kind, "level": level, "text": text} if kind == "heading"
                    else {"kind": kind, "text": text})
    return {"lang": lang, "layout": layout, "blocks": keep, "furniture": [HEADER[lang]],
            "footnote_markers": footnotes}


# --- producers -----------------------------------------------------------------

def chrome(blocks, lang, layout, family, figure, pdf):
    # Chrome has no footnotes: they are left out of the source and the truth.
    two = layout == "two"
    body = []
    for kind, level, text in blocks:
        t = html.escape(text)
        if kind == "heading":
            body.append(f"<h{level}>{t}</h{level}>")
        elif kind == "paragraph":
            body.append(f"<p>{t}</p>")
        elif kind == "image":
            body.append(f'<figure><img src="file://{figure}"></figure>')
    counter = "counter(page, bengali)" if lang == "bn" else "counter(page)"
    doc = f"""<!doctype html><html lang="{LANG[lang][1]}"><meta charset="utf-8"><style>
@page {{ size: A5; margin: 20mm 14mm;
  @top-center {{ content: "{HEADER[lang]}"; font-family: '{family}'; font-size: 8pt; }}
  @bottom-center {{ content: {counter}; font-family: '{family}'; font-size: 9pt; }} }}
body {{ font-family: '{family}', serif; font-size: {10.5 if two else 12}pt; line-height: 1.5; margin: 0;
  {"text-align: justify; hyphens: auto;" if two else "text-align: left;"} }}
{".cols { column-count: 2; column-gap: 7mm; }" if two else ""}
h1 {{ font-size: 18pt; margin: 0 0 8pt; }} h2 {{ font-size: 13.5pt; margin: 12pt 0 6pt; }}
p {{ margin: 0; text-indent: {"1.5em" if two else "0"}; {"" if two else "margin-bottom: 0.6em;"} }}
figure {{ margin: 6pt 0; }} img {{ width: 100%; }}
</style><body>{body[0]}<div class="{"cols" if two else ""}">{"".join(body[1:])}</div></body></html>"""
    with tempfile.NamedTemporaryFile("w", suffix=".html", delete=False, encoding="utf-8") as f:
        f.write(doc)
    subprocess.run(["google-chrome", "--headless", "--no-sandbox", "--disable-gpu", "--no-pdf-header-footer",
                    f"--print-to-pdf={pdf}", "file://" + f.name], check=True, capture_output=True, timeout=120)
    return False


def writer(blocks, lang, layout, family, figure, pdf):
    """A Word document (.docx) printed by LibreOffice Writer."""
    from docx import Document
    from docx.enum.text import WD_ALIGN_PARAGRAPH
    from docx.oxml import OxmlElement
    from docx.oxml.ns import qn
    from docx.shared import Mm, Pt

    two = layout == "two"
    d = Document()
    sec = d.sections[0]
    sec.page_width, sec.page_height = Mm(148), Mm(210)
    sec.left_margin = sec.right_margin = Mm(14)
    sec.top_margin = sec.bottom_margin = Mm(20)
    if two:
        cols = OxmlElement("w:cols")
        cols.set(qn("w:num"), "2")
        cols.set(qn("w:space"), "400")
        sec._sectPr.append(cols)

    def font(run, size):
        run.font.name = family
        run.font.size = Pt(size)
        rpr = run._element.get_or_add_rPr()
        fonts = rpr.find(qn("w:rFonts"))
        for a in ("w:ascii", "w:hAnsi", "w:cs", "w:eastAsia"):
            fonts.set(qn(a), family)
        cs = OxmlElement("w:szCs")
        cs.set(qn("w:val"), str(int(size * 2)))
        rpr.append(cs)

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

    for kind, level, text in blocks:
        if kind == "image":
            d.add_paragraph().add_run().add_picture(figure, width=Mm(55 if two else 110))
            continue
        if kind == "footnote":
            continue  # python-docx has no footnotes: left out of the truth too
        p = d.add_paragraph()
        size = {1: 18, 2: 13.5}.get(level, 10.5 if two else 12) if kind == "heading" else (10.5 if two else 12)
        r = p.add_run(text)
        font(r, size)
        r.bold = kind == "heading"
        pf = p.paragraph_format
        pf.space_after = Pt(6 if kind == "heading" or not two else 0)
        if kind == "paragraph":
            pf.alignment = WD_ALIGN_PARAGRAPH.JUSTIFY if two else WD_ALIGN_PARAGRAPH.LEFT
            if two:
                pf.first_line_indent = Pt(16)
    if two:
        settings = d.settings.element
        hy = OxmlElement("w:autoHyphenation")
        hy.set(qn("w:val"), "true")
        settings.append(hy)
    work = tempfile.mkdtemp()
    src = os.path.join(work, os.path.basename(pdf)[:-4] + ".docx")
    d.save(src)
    subprocess.run(["soffice", "--headless", "--convert-to", "pdf", "--outdir", work, src],
                   check=True, capture_output=True, timeout=180)
    shutil.move(src[:-5] + ".pdf", pdf)
    return False


def tex_escape(s):
    return s.replace("\\", r"\textbackslash{}").replace("&", r"\&").replace("%", r"\%").replace("#", r"\#")


def latex(engine):
    def make(blocks, lang, layout, family, figure, pdf):
        two = layout == "two"
        fname = dict(FONTS[lang])[family]
        fdir = os.path.dirname(font_path(fname)) + "/"
        renderer = ",Renderer=HarfBuzz" if engine == "lualatex" else ""
        script = "Bengali" if lang == "bn" else "Devanagari"
        lines = [
            rf"\documentclass[{'10pt,twocolumn' if two else '12pt'}]{{article}}",
            r"\usepackage[a5paper,margin=16mm,headsep=6mm]{geometry}",
            r"\usepackage{fontspec}\usepackage{graphicx}",
            rf"\setmainfont{{{fname}}}[Path={fdir},Script={script}{renderer}]",
            rf"\usepackage{{polyglossia}}\setdefaultlanguage{{{LANG[lang][0]}}}",
            rf"\newfontfamily\{LANG[lang][0]}font{{{fname}}}[Path={fdir},Script={script}{renderer}]",
            r"\usepackage{fancyhdr}\pagestyle{fancy}\fancyhf{}\renewcommand{\headrulewidth}{0pt}",
            rf"\fancyhead[C]{{\footnotesize {HEADER[lang]}}}\fancyfoot[C]{{\thepage}}",
            r"\setlength{\parindent}{" + ("1.2em" if two else "0pt") + r"}",
            r"\setlength{\parskip}{" + ("0pt" if two else "6pt") + r"}",
            r"\begin{document}",
            "" if two else r"\raggedright",
        ]
        for i, (kind, level, text) in enumerate(blocks):
            t = tex_escape(text)
            nxt = blocks[i + 1] if i + 1 < len(blocks) else None
            note = two and nxt and nxt[0] == "footnote"
            if kind == "heading":
                lines.append((r"\section*{" if level == 1 else r"\subsection*{") + t + "}")
            elif kind == "paragraph":
                lines.append(t + (r"\footnote{" + tex_escape(nxt[2]) + "}" if note else "") + "\n")
            elif kind == "image":
                lines.append(r"\noindent\includegraphics[width=\linewidth]{" + figure + "}\n")
        lines.append(r"\end{document}")
        work = tempfile.mkdtemp()
        tex = os.path.join(work, "book.tex")
        open(tex, "w", encoding="utf-8").write("\n".join(lines))
        r = subprocess.run([engine, "-interaction=nonstopmode", "-halt-on-error", "book.tex"], cwd=work,
                           capture_output=True, timeout=300)
        if not os.path.exists(os.path.join(work, "book.pdf")):
            raise RuntimeError(f"{engine} failed:\n" + r.stdout.decode(errors="replace")[-1500:])
        shutil.move(os.path.join(work, "book.pdf"), pdf)
        return two
    return make


def typst_book(blocks, lang, layout, family, figure, pdf):
    import typst

    two = layout == "two"

    def q(s):
        return s.replace("\\", "\\\\").replace("#", "\\#").replace("*", "\\*").replace("_", "\\_") \
            .replace("[", "\\[").replace("]", "\\]").replace("$", "\\$").replace("@", "\\@").replace("<", "\\<")

    src = [
        f'#set page(width: 148mm, height: 210mm, margin: (x: 14mm, y: 20mm), columns: {2 if two else 1},',
        f'  header: align(center, text(size: 8pt)[{q(HEADER[lang])}]),',
        '  footer: context align(center, text(size: 9pt, counter(page).display())))',
        f'#set text(font: "{family}", size: {10.5 if two else 12}pt, lang: "{LANG[lang][1]}", hyphenate: {"true" if two else "false"})',
        f'#set par(justify: {"true" if two else "false"}, first-line-indent: {"1.2em" if two else "0em"}, spacing: {"0.65em" if two else "1em"})',
        '#show heading.where(level: 1): set text(size: 18pt)',
        '#show heading.where(level: 2): set text(size: 13.5pt)',
    ]
    for i, (kind, level, text) in enumerate(blocks):
        nxt = blocks[i + 1] if i + 1 < len(blocks) else None
        if kind == "heading":
            src.append("=" * level + " " + q(text))
        elif kind == "paragraph":
            note = two and nxt and nxt[0] == "footnote"
            src.append(q(text) + (f"#footnote[{q(nxt[2])}]" if note else "") + "\n")
        elif kind == "image":
            src.append(f'#image("{os.path.basename(figure)}", width: 100%)\n')
    work = tempfile.mkdtemp()
    shutil.copy(figure, work)
    path = os.path.join(work, "book.typ")
    open(path, "w", encoding="utf-8").write("\n".join(src))
    typst.compile(path, output=pdf, font_paths=FONT_DIRS, root=work)
    return two


PRODUCERS = [("chrome", chrome), ("writer", writer), ("xelatex", latex("xelatex")),
             ("lualatex", latex("lualatex")), ("typst", typst_book)]


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else None
    os.makedirs(OUT, exist_ok=True)
    figure = os.path.join(tempfile.mkdtemp(), "figure.png")
    make_figure(figure)
    for lang in ("bn", "hi"):
        source = parse(os.path.join(SRC, f"{lang}-book.txt"))
        for producer, make in PRODUCERS:
            if only and only != producer:
                continue
            for layout in ("one", "two"):
                # Second designs in two columns for Chrome and XeLaTeX; Kalpurush
                # through Word, the usual way it reaches a PDF.
                designs = FONTS[lang][:1]
                if layout == "two" and producer in ("chrome", "xelatex"):
                    designs = FONTS[lang][:2]
                if producer == "writer":
                    designs = FONTS[lang][:1] + FONTS[lang][2:3]
                for i, (family, _) in enumerate(designs):
                    blocks = with_figure(source) if layout == "two" else source
                    blocks_in = [b for b in blocks if b[0] != "footnote"] if layout == "one" else blocks
                    stem = f"{lang}.{producer}.{layout}" + ("" if i == 0 else "." + family.split()[0].lower())
                    pdf = os.path.join(OUT, stem + ".pdf")
                    try:
                        footnotes = make(blocks_in, lang, layout, family, figure, pdf)
                    except Exception as e:  # a missing producer must not stop the rest
                        print(f"{stem}: FAILED {e}")
                        continue
                    t = truth(blocks_in, lang, layout, footnotes)
                    json.dump(t, open(os.path.join(OUT, stem + ".json"), "w", encoding="utf-8"),
                              ensure_ascii=False, indent=1)
                    print(f"{stem}.pdf  {os.path.getsize(pdf)} bytes")


if __name__ == "__main__":
    main()
