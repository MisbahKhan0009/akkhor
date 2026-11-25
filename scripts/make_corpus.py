#!/usr/bin/env python3
"""Builds the PDF accuracy corpus in tests/corpus/pdf from the ground-truth
texts in tests/corpus/*.txt.

Each text is printed by two producers (Chrome and LibreOffice) with the
Noto Serif fonts from fonts/ ({stem}.{producer}.pdf) and with a second
design from tests/fonts/ ({stem}.{font}.{producer}.pdf). Every PDF is also
saved with its ToUnicode maps and ActualText spans removed (*.raw.pdf),
which forces the engine to recover the text from the glyphs alone.
`--raw-only` regenerates just those from the existing PDFs. bn.txt is also printed in Bijoy
(converted with the poriborton crate) in a font named SutonnyMJ
(bn.bijoy.{producer}.pdf).

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
    make_fake_sutonny(os.path.join(dest, "FakeSutonnyMJ.ttf"))
    subprocess.run(["fc-cache", "-f", dest], check=False, capture_output=True)


def make_fake_sutonny(path):
    """A font *named* SutonnyMJ, for Bijoy PDFs. Its shapes are Noto Serif's
    Latin (SutonnyMJ itself is not freely licensed); only the name and the
    Latin-1 text matter to the text layer. Never committed, only installed."""
    from fontTools.ttLib import TTFont

    font = TTFont(os.path.join(ROOT, "fonts", "NotoSerif-Regular.ttf"))
    for rec in font["name"].names:
        if rec.nameID in (1, 4, 16, 21):
            rec.string = "SutonnyMJ"
        elif rec.nameID == 6:
            rec.string = "SutonnyMJ-Regular"
    font.save(path)


def to_bijoy(text):
    """Unicode Bangla to Bijoy with the poriborton crate (an independent
    implementation from Akkhor's own converter)."""
    out = subprocess.run(
        ["cargo", "run", "-q", "-p", "akkhor-flow", "--example", "to_bijoy"],
        input=text.encode(), capture_output=True, check=True, cwd=ROOT,
    )
    return out.stdout.decode()


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
    """Writes `src` without any text information: every /ToUnicode map and
    every /ActualText span removed, so only the glyphs are left. Returns the
    number of maps and spans removed."""
    import pikepdf

    pdf = pikepdf.open(src)
    count = 0
    for obj in pdf.objects:
        if isinstance(obj, pikepdf.Dictionary) and "/ToUnicode" in obj:
            del obj["/ToUnicode"]
            count += 1

    def clean(stream_owner):
        nonlocal count
        ops = []
        for operands, op in pikepdf.parse_content_stream(stream_owner):
            if str(op) == "BDC" and len(operands) == 2 and isinstance(operands[1], pikepdf.Dictionary) \
                    and "/ActualText" in operands[1]:
                props = pikepdf.Dictionary({k: v for k, v in operands[1].items() if k != "/ActualText"})
                operands = [operands[0], props]
                count += 1
            ops.append((operands, op))
        return pikepdf.unparse_content_stream(ops)

    for page in pdf.pages:
        page.Contents = pdf.make_stream(clean(page))
        for _, xobj in page.Resources.get("/XObject", {}).items():
            if xobj.get("/Subtype") == "/Form":
                xobj.write(clean(xobj))
    pdf.save(dst, deterministic_id=True)
    return count


def raw_only():
    """Regenerates every *.raw.pdf from its as-produced PDF, without
    printing anything again."""
    for name in sorted(os.listdir(OUT)):
        if name.endswith(".pdf") and not name.endswith(".raw.pdf"):
            n = strip_tounicode(os.path.join(OUT, name), os.path.join(OUT, name[:-4] + ".raw.pdf"))
            print(f"{name[:-4]}.raw.pdf  ({n} maps and spans removed)")


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
        if stem == "bn":
            # Set D: the same text typed in Bijoy, in a font named SutonnyMJ.
            bijoy = to_bijoy(text)
            html_path = os.path.join(work, "bn.bijoy.html")
            open(html_path, "w", encoding="utf-8").write(page_html(bijoy, "SutonnyMJ"))
            for producer, make in (("chrome", chrome), ("libreoffice", libreoffice)):
                pdf = os.path.join(OUT, f"bn.bijoy.{producer}.pdf")
                make(html_path, pdf)
                n = strip_tounicode(pdf, os.path.join(OUT, f"bn.bijoy.{producer}.raw.pdf"))
                print(f"bn.bijoy.{producer}.pdf  ({os.path.getsize(pdf)} bytes, {n} ToUnicode maps)")
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
    import sys

    raw_only() if "--raw-only" in sys.argv else main()
