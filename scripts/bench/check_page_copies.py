#!/usr/bin/env python3
"""Pages copied by Akkhor (PdfDocument::write_pages) against the originals,
in MuPDF, PDFium and Poppler: does the copy open, check clean (qpdf), give
the same text and the same pixels as the original pages?

  cargo build --release -p akkhor-pdf --features write --example copy_pages
  python3 scripts/bench/check_page_copies.py [--csv data/page_copies.csv]

Per file it copies every page, last first (files of up to 4 pages), or
the last page and then the first, and compares each copied page with its
original. Pixels: rendered at 72 dpi; a pixel differs when a channel is
off by more than 8.
"""
import argparse, csv, glob, io, os, subprocess, sys, tempfile
import fitz, pypdfium2 as pdfium
from PIL import Image, ImageChops

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.path.join(ROOT, "target/release/examples/copy_pages")


def norm(t):
    return " ".join(t.split())


def mupdf(path, pages):
    d = fitz.open(path)
    return {i: (norm(d[i].get_text()), Image.open(io.BytesIO(d[i].get_pixmap(dpi=72).tobytes("png"))).convert("RGB")) for i in pages}


def pdfium_pages(path, pages):
    d = pdfium.PdfDocument(path)
    out = {}
    for i in pages:
        p = d[i]
        text = norm(p.get_textpage().get_text_range())
        out[i] = (text, p.render(scale=1).to_pil().convert("RGB"))
    return out


def poppler(path, pages):
    out = {}
    with tempfile.TemporaryDirectory() as tmp:
        for i in (p + 1 for p in pages):
            t = subprocess.run(["pdftotext", "-q", "-f", str(i), "-l", str(i), path, "-"], capture_output=True).stdout.decode("utf-8", "replace")
            subprocess.run(["pdftoppm", "-q", "-r", "72", "-f", str(i), "-l", str(i), "-png", "-singlefile", path, f"{tmp}/p"], check=False)
            img = Image.open(f"{tmp}/p.png").convert("RGB") if os.path.exists(f"{tmp}/p.png") else None
            if img: os.remove(f"{tmp}/p.png")
            out[i - 1] = (norm(t), img)
    return out


def differing(a, b):
    if a is None or b is None or a.size != b.size:
        return -1
    d = ImageChops.difference(a, b).point(lambda v: 255 if v > 8 else 0).convert("L")
    return sum(1 for v in d.tobytes() if v)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--csv")
    args = ap.parse_args()
    files = sorted(glob.glob(f"{ROOT}/tests/**/*.pdf", recursive=True))
    rows, bad = [], 0
    with tempfile.TemporaryDirectory() as tmp:
        for f in files:
            rel = os.path.relpath(f, ROOT)
            try:
                n = fitz.open(f).page_count
            except Exception:
                continue  # damaged on purpose: MuPDF cannot open it either
            if n == 0:
                continue
            out = f"{tmp}/copy.pdf"
            # Short files: every page, last first. Longer: the last, then the first.
            chosen = list(range(n - 1, -1, -1)) if n <= 4 else [n - 1, 0]
            r = subprocess.run([BIN, f, out, ",".join(str(i + 1) for i in chosen)], capture_output=True, text=True)
            if r.returncode != 0:
                rows.append({"file": rel, "result": "refused " + r.stdout.strip()})
                print(f"refused  {rel}  {r.stdout.strip()}")
                continue
            size_in, size_out = os.path.getsize(f), os.path.getsize(out)
            q = subprocess.run(["qpdf", "--check", out], capture_output=True, text=True)
            qpdf_ok = q.returncode == 0 and "No syntax or stream encoding errors" in q.stdout
            row = {"file": rel, "pages": n, "bytes_in": size_in, "bytes_out": size_out, "qpdf_clean": qpdf_ok}
            pairs = [(o, c) for c, o in enumerate(chosen)]
            if fitz.open(out).page_count != len(chosen):
                rows.append({"file": rel, "result": "wrong page count"})
                bad += 1
                continue
            for name, reader in [("mupdf", mupdf), ("pdfium", pdfium_pages), ("poppler", poppler)]:
                try:
                    orig, copy = reader(f, sorted(set(chosen))), reader(out, list(range(len(chosen))))
                    same_text = all(orig[o][0] == copy[c][0] for o, c in pairs)
                    px = [differing(orig[o][1], copy[c][1]) for o, c in pairs]
                    row[f"{name}_text"] = same_text
                    row[f"{name}_pixels_off"] = max(px) if min(px) >= 0 else -1
                except Exception as e:
                    row[f"{name}_text"] = False
                    row[f"{name}_pixels_off"] = -1
                    print(rel, name, "error", e, file=sys.stderr)
            ok = qpdf_ok and all(row[f"{x}_text"] and row[f"{x}_pixels_off"] == 0 for x in ("mupdf", "pdfium", "poppler"))
            row["result"] = "same" if ok else "differs"
            bad += not ok
            rows.append(row)
            print(f"{row['result']:8} {rel}  " + " ".join(f"{k}={v}" for k, v in row.items() if k not in ("file", "result")))
    if args.csv:
        keys = ["file", "pages", "bytes_in", "bytes_out", "qpdf_clean"] + [f"{x}_{y}" for x in ("mupdf", "pdfium", "poppler") for y in ("text", "pixels_off")] + ["result"]
        with open(args.csv, "w", newline="") as fh:
            w = csv.DictWriter(fh, keys)
            w.writeheader()
            for r in rows:
                w.writerow({k: r.get(k, "") for k in keys})
    print(f"{len(rows)} files, {bad} differ")


if __name__ == "__main__":
    main()
