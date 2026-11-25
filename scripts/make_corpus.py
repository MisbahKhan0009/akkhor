#!/usr/bin/env python3
"""Builds the PDF accuracy corpus in tests/corpus/pdf from the ground-truth
texts in tests/corpus/*.txt.

Each text is printed by two producers (Chrome and LibreOffice) with the
Noto Serif fonts from fonts/ ({stem}.{producer}.pdf) and with a second
design from tests/fonts/ ({stem}.{font}.{producer}.pdf). Every PDF is also
saved with its ToUnicode maps removed (*.raw.pdf), which forces the engine
to recover the text from the glyphs alone.

Needs google-chrome and soffice on PATH. Run from the repository root:
    python3 scripts/make_corpus.py
"""

import html
import os
import re
import shutil
import subprocess
import tempfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
CORPUS = os.path.join(ROOT, "tests", "corpus")
OUT = os.path.join(CORPUS, "pdf")
FONTS = [os.path.join(ROOT, "fonts"), os.path.join(ROOT, "tests", "fonts")]

# (file key, family) per language; key None is the default Noto design.
FAMILIES = {
    "bn": [(None, "Noto Serif Bengali"), ("hind", "Hind Siliguri")],
    "hi": [(None, "Noto Serif Devanagari"), ("mukta", "Mukta")],
}


def install_fonts():
    """LibreOffice finds fonts through fontconfig."""
    dest = os.path.expanduser("~/.local/share/fonts/akkhor")
    os.makedirs(dest, exist_ok=True)
    for d in FONTS:
        for f in os.listdir(d):
            if f.endswith(".ttf"):
                shutil.copy(os.path.join(d, f), dest)
    subprocess.run(["fc-cache", "-f", dest], check=False, capture_output=True)


def page_html(text, family):
    paras = "\n".join(f"<p>{html.escape(line)}</p>" for line in text.splitlines() if line.strip())
    return f"""<!doctype html><html><head><meta charset="utf-8"><style>
body {{ font-family: '{family}', serif; font-size: 14pt; line-height: 1.6; margin: 2cm; }}
p {{ margin: 0 0 0.6em; }}
</style></head><body>
{paras}
</body></html>"""


def chrome(html_path, pdf_path):
    subprocess.run(
        ["google-chrome", "--headless", "--no-sandbox", "--disable-gpu", "--no-pdf-header-footer",
         f"--print-to-pdf={pdf_path}", "file://" + html_path],
        check=True, capture_output=True, timeout=120,
    )


def libreoffice(html_path, pdf_path):
    outdir = tempfile.mkdtemp()
    subprocess.run(
        ["soffice", "--headless", "--convert-to", "pdf:writer_web_pdf_Export", "--outdir", outdir, html_path],
        check=True, capture_output=True, timeout=180,
    )
    produced = os.path.join(outdir, os.path.splitext(os.path.basename(html_path))[0] + ".pdf")
    shutil.move(produced, pdf_path)


def strip_tounicode(src, dst):
    """Blanks out every /ToUnicode reference, keeping byte offsets intact."""
    data = open(src, "rb").read()
    count = 0

    def blank(m):
        nonlocal count
        count += 1
        return b" " * len(m.group(0))

    data = re.sub(rb"/ToUnicode\s+\d+\s+\d+\s+R", blank, data)
    open(dst, "wb").write(data)
    return count


def main():
    install_fonts()
    os.makedirs(OUT, exist_ok=True)
    work = tempfile.mkdtemp()
    for name in sorted(os.listdir(CORPUS)):
        if not name.endswith(".txt"):
            continue
        stem = name[:-4]
        lang = stem.split("-")[0]
        text = open(os.path.join(CORPUS, name), encoding="utf-8").read()
        for key, family in FAMILIES[lang]:
            base = stem if key is None else f"{stem}.{key}"
            html_path = os.path.join(work, base + ".html")
            open(html_path, "w", encoding="utf-8").write(page_html(text, family))
            for producer, make in (("chrome", chrome), ("libreoffice", libreoffice)):
                pdf = os.path.join(OUT, f"{base}.{producer}.pdf")
                make(html_path, pdf)
                n = strip_tounicode(pdf, os.path.join(OUT, f"{base}.{producer}.raw.pdf"))
                print(f"{base}.{producer}.pdf  ({os.path.getsize(pdf)} bytes, {n} ToUnicode maps)")


if __name__ == "__main__":
    main()
