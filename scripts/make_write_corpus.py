#!/usr/bin/env python3
"""Makes tests/corpus/write: PDFs that stress copying pages.

- inherited.pdf: MediaBox, Resources and Rotate set on the page tree, not
  the pages; a page tree two levels deep.
- objstreams.pdf: a corpus book rewritten with objects inside object
  streams and a cross-reference stream (qpdf).
- annots.pdf: a highlight, a sticky note, a free-text note, a link to
  another page, and a drawn form field that is the kid of a parent field.
- encrypted.pdf: an empty user password (opens without asking), which a
  copy must still refuse.
"""
import os, subprocess
import fitz

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tests/corpus/write")
BOOK = os.path.join(ROOT, "tests/corpus/books/pdf/bn.chrome.two.pdf")


def write_pdf(path, objects):
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for i, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += f"{i} 0 obj\n".encode() + body + b"\nendobj\n"
    start = len(out)
    out += f"xref\n0 {len(objects) + 1}\n0000000000 65535 f\r\n".encode()
    for o in offsets:
        out += f"{o:010} 00000 n\r\n".encode()
    out += f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n".encode()
    open(path, "wb").write(out)


def stream(data, extra=b""):
    return b"<< /Length %d %s >>\nstream\n" % (len(data), extra) + data + b"\nendstream"


def inherited():
    page = lambda text: stream(b"BT /F1 24 Tf 72 700 Td (%s) Tj ET 0 0 1 rg 72 600 200 50 re f" % text)
    write_pdf(f"{OUT}/inherited.pdf", [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 3 /MediaBox [0 0 400 800] /Resources << /Font << /F1 4 0 R >> >> >>",
        b"<< /Type /Pages /Parent 2 0 R /Kids [5 0 R 6 0 R 7 0 R] /Count 3 /Rotate 90 >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Type /Page /Parent 3 0 R /Contents 8 0 R >>",
        b"<< /Type /Page /Parent 3 0 R /Contents 9 0 R /Rotate 0 >>",
        b"<< /Type /Page /Parent 3 0 R /Contents 10 0 R /MediaBox [0 0 300 300] >>",
        page(b"Page one"),
        page(b"Page two"),
        page(b"Page three"),
    ])


def annots():
    d = fitz.open()
    for n in range(3):
        p = d.new_page(width=420, height=595)
        p.insert_text((50, 80), f"Page {n + 1}: annotations", fontsize=16)
    p = d[1]
    p.add_highlight_annot(fitz.Rect(48, 64, 260, 86))
    p.add_text_annot((300, 120), "a sticky note")
    p.add_freetext_annot(fitz.Rect(50, 150, 300, 200), "a free-text note", fontsize=12, text_color=(0.6, 0, 0))
    p.insert_link({"kind": fitz.LINK_GOTO, "from": fitz.Rect(50, 220, 200, 240), "page": 2})
    w = fitz.Widget()
    w.field_type = fitz.PDF_WIDGET_TYPE_TEXT
    w.field_name = "name"
    w.field_value = "filled in"
    w.rect = fitz.Rect(50, 260, 250, 285)
    wx = p.add_widget(w).xref
    # Make the widget the kid of a parent field, which holds the type:
    # a copy must carry the type and the full name ("person.name") over.
    parent = d.get_new_xref()
    d.update_object(parent, f"<< /T (person) /FT /Tx /Kids [{wx} 0 R] >>")
    d.xref_set_key(wx, "Parent", f"{parent} 0 R")
    d.xref_set_key(wx, "FT", "null")
    form = d.pdf_catalog()
    d.xref_set_key(form, "AcroForm/Fields", f"[{parent} 0 R]")
    d.save(f"{OUT}/annots.pdf")


def main():
    os.makedirs(OUT, exist_ok=True)
    inherited()
    annots()
    subprocess.run(["qpdf", "--object-streams=generate", BOOK, f"{OUT}/objstreams.pdf"], check=True)
    subprocess.run(["qpdf", "--encrypt", "", "owner", "256", "--", BOOK, f"{OUT}/encrypted.pdf"], check=True)


if __name__ == "__main__":
    main()
