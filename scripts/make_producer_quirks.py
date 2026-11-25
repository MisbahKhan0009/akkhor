#!/usr/bin/env python3
"""Derives corpus PDFs that reproduce quirks seen in real-world Bangla PDFs
(found on books measured locally, which cannot be committed):

  *.fakebold.raw.pdf  every text block drawn a second time on itself in
                      stroke mode, the way some tools fake bold; a reader
                      that keeps both copies doubles every letter
                      (ককপিিপররাাইইটট).
  *.rewritten.raw.pdf every embedded TrueType program's left side bearings
                      set to zero and the head flag that keeps outlines at
                      their lsb cleared, as font rewriters (Ghostscript) do;
                      outlines move sideways and exact outline matching
                      fails.

Both start from the .raw.pdf files (no ToUnicode, no ActualText), so the
text can only come from recovering the glyphs. Run from the repository
root:  python3 scripts/make_producer_quirks.py
"""

import io
import os

import pikepdf
from fontTools.ttLib import TTFont

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
PDF = os.path.join(ROOT, "tests", "corpus", "pdf")
SOURCES = ["bn.chrome.raw.pdf", "hi.libreoffice.raw.pdf", "bn.hind.libreoffice.raw.pdf"]


def fake_bold(src, dst):
    pdf = pikepdf.open(src)
    for page in pdf.pages:
        ops = list(pikepdf.parse_content_stream(page))
        out, block = [], None
        for operands, op in ops:
            out.append((operands, op))
            name = str(op)
            if name == "BT":
                block = [(operands, op), ([1], pikepdf.Operator("Tr"))]
            elif block is not None:
                block.append((operands, op))
                if name == "ET":
                    out.extend(block)
                    block = None
        page.Contents = pdf.make_stream(pikepdf.unparse_content_stream(out))
    pdf.save(dst, deterministic_id=True)


def rewritten_fonts(src, dst):
    pdf = pikepdf.open(src)
    n = 0
    for obj in pdf.objects:
        if isinstance(obj, pikepdf.Dictionary) and "/FontFile2" in obj:
            stream = obj.FontFile2
            font = TTFont(io.BytesIO(stream.read_bytes()))
            hmtx = font["hmtx"].metrics
            for g in list(hmtx):
                adv, _ = hmtx[g]
                hmtx[g] = (adv, 0)
            font["head"].flags &= ~(1 << 1)
            buf = io.BytesIO()
            font.save(buf)
            stream.write(buf.getvalue())
            n += 1
    pdf.save(dst, deterministic_id=True)
    return n


def main():
    for name in SOURCES:
        src = os.path.join(PDF, name)
        stem = name[: -len(".raw.pdf")]
        fake_bold(src, os.path.join(PDF, f"{stem}.fakebold.raw.pdf"))
        n = rewritten_fonts(src, os.path.join(PDF, f"{stem}.rewritten.raw.pdf"))
        print(f"{stem}: fakebold, rewritten ({n} fonts)")


if __name__ == "__main__":
    main()
