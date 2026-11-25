#!/usr/bin/env python3
"""One engine, one measurement, one JSON line: the other engines' side of
the comparison (Akkhor's is `akkhor-bench`). Run by compare_engines.py in a
fresh process per measurement, so memory and crashes stay isolated.

    engines.py ENGINE text    FILE
    engines.py ENGINE speed   FILE PAGE SCALE RUNS
    engines.py ENGINE mem     FILE PAGE SCALE
    engines.py ENGINE survive FILE

ENGINE: mupdf (PyMuPDF), pdfium (pypdfium2, the engine in Chrome),
poppler (its command-line tools; timings include process start-up, which
compare_engines.py subtracts using the tools' --version run).
"""

import json
import os
import resource
import subprocess
import sys
import tempfile
import time

POPPLER = "/usr/bin"  # the distribution's tools, not wrappers on PATH


def status_kb(field):
    for line in open("/proc/self/status"):
        if line.startswith(field):
            return int(line.split()[1])
    return 0


def reset_peak():
    """Linux: resets VmHWM, so the peak excludes the interpreter's start-up."""
    try:
        with open("/proc/self/clear_refs", "w") as f:
            f.write("5")
    except OSError:
        pass


def now():
    return time.perf_counter()


def ms(t):
    return (time.perf_counter() - t) * 1e3


# --- MuPDF -----------------------------------------------------------------

def mupdf():
    import pymupdf
    pymupdf.TOOLS.mupdf_display_errors(False)

    class E:
        name = "MuPDF " + pymupdf.VersionFitz

        def open(self, path):
            return pymupdf.open(path)

        def count(self, doc):
            return doc.page_count

        def render(self, doc, page, scale):
            # Four bytes a pixel, like Akkhor and Android's ARGB_8888
            # bitmaps (three-byte RGB would make MuPDF look 25% lighter).
            pix = doc[page].get_pixmap(matrix=pymupdf.Matrix(scale, scale), alpha=True)
            return pix.width

        def text(self, doc, page):
            return doc[page].get_text()

    return E()


# --- PDFium ----------------------------------------------------------------

def pdfium():
    import pypdfium2 as pdfium

    class E:
        name = "PDFium " + str(pdfium.PDFIUM_INFO).split()[0] if hasattr(pdfium, "PDFIUM_INFO") else "PDFium"

        def open(self, path):
            return pdfium.PdfDocument(path)

        def count(self, doc):
            return len(doc)

        def render(self, doc, page, scale):
            # Four bytes a pixel, as for every engine (default is BGR).
            bitmap = doc[page].render(scale=scale, force_bitmap_format=pdfium.raw.FPDFBitmap_BGRA)
            return bitmap.width

        def text(self, doc, page):
            return doc[page].get_textpage().get_text_range()

    return E()


# --- Poppler ---------------------------------------------------------------

def run(args, timeout=60):
    return subprocess.run(args, capture_output=True, timeout=timeout)


def poppler_cmd(cmd, args):
    if cmd == "text":
        t = now()
        out = run([f"{POPPLER}/pdftotext", "-enc", "UTF-8", args[0], "-"])
        total = ms(t)
        pages = out.stdout.decode("utf-8", "replace").split("\f")
        return {"ms": total, "pages": pages}
    if cmd == "speed":
        path, page, scale, runs = args[0], int(args[1]), float(args[2]), int(args[3])
        res = {"open_ms": [], "first_render_ms": [], "warm_render_ms": [], "startup_ms": []}
        with tempfile.TemporaryDirectory() as d:
            for _ in range(runs):
                t = now(); run([f"{POPPLER}/pdfinfo", "-v"]); res["startup_ms"].append(ms(t))
                t = now(); run([f"{POPPLER}/pdfinfo", path]); res["open_ms"].append(ms(t))
                for key in ("first_render_ms", "warm_render_ms"):
                    t = now()
                    run([f"{POPPLER}/pdftoppm", "-f", str(page + 1), "-l", str(page + 1),
                         "-r", str(72 * scale), path, os.path.join(d, "p")])
                    res[key].append(ms(t))
        return res
    if cmd == "mem":
        path, page, scale = args[0], int(args[1]), float(args[2])
        with tempfile.TemporaryDirectory() as d:
            before = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
            run([f"{POPPLER}/pdftoppm", "-v"])
            base = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
            # A fresh process for the measured run, so its peak is its own.
            code = (
                "import resource,subprocess,sys;"
                f"subprocess.run(sys.argv[1:],capture_output=True);"
                "print(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)"
            )
            out = run([sys.executable, "-c", code, f"{POPPLER}/pdftoppm", "-f", str(page + 1),
                       "-l", str(page + 1), "-r", str(72 * scale), path, os.path.join(d, "p")])
            peak = int(out.stdout.strip() or 0)
            return {"rss_before_kb": base if base > before else before, "peak_kb": peak}
    if cmd == "survive":
        try:
            with tempfile.TemporaryDirectory() as d:
                r = run([f"{POPPLER}/pdftoppm", "-f", "1", "-l", "1", "-r", "36", args[0],
                         os.path.join(d, "p")], timeout=20)
                t = run([f"{POPPLER}/pdftotext", "-f", "1", "-l", "1", args[0], "-"], timeout=20)
        except subprocess.TimeoutExpired:
            return {"ok": False, "hang": True}
        crashed = r.returncode < 0 or t.returncode < 0
        return {"ok": r.returncode == 0 and t.returncode == 0, "crash": crashed}
    raise SystemExit(f"unknown command {cmd}")


# --- shared ----------------------------------------------------------------

def library_cmd(engine, cmd, args):
    if cmd == "text":
        t = now()
        doc = engine.open(args[0])
        pages = [engine.text(doc, i) for i in range(engine.count(doc))]
        return {"ms": ms(t), "pages": pages}
    if cmd == "speed":
        path, page, scale, runs = args[0], int(args[1]), float(args[2]), int(args[3])
        res = {"open_ms": [], "first_render_ms": [], "warm_render_ms": []}
        for _ in range(runs):
            t = now(); doc = engine.open(path); res["open_ms"].append(ms(t))
            t = now(); engine.render(doc, page, scale); res["first_render_ms"].append(ms(t))
            t = now(); engine.render(doc, page, scale); res["warm_render_ms"].append(ms(t))
            del doc
        return res
    if cmd == "mem":
        path, page, scale = args[0], int(args[1]), float(args[2])
        reset_peak()
        before = status_kb("VmRSS:")
        doc = engine.open(path)
        engine.render(doc, page, scale)
        return {"rss_before_kb": before, "peak_kb": status_kb("VmHWM:")}
    if cmd == "survive":
        try:
            doc = engine.open(args[0])
            # Opened means a page shown: a document with no pages shows
            # nothing (same rule for every engine).
            if engine.count(doc) == 0:
                return {"ok": False, "error": "no pages"}
            engine.render(doc, 0, 0.5)
            engine.text(doc, 0)
            return {"ok": True}
        except Exception as e:  # a graceful error, not a crash
            return {"ok": False, "error": str(e)[:200]}
    raise SystemExit(f"unknown command {cmd}")


def main():
    name, cmd, args = sys.argv[1], sys.argv[2], sys.argv[3:]
    if name == "poppler":
        out = poppler_cmd(cmd, args)
    else:
        engine = {"mupdf": mupdf, "pdfium": pdfium}[name]()
        out = library_cmd(engine, cmd, args)
    print(json.dumps(out, ensure_ascii=False))


if __name__ == "__main__":
    main()
