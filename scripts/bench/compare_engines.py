#!/usr/bin/env python3
"""Compares Akkhor with MuPDF, PDFium and Poppler on the same files and
writes the results to docs/case-study/data/.

    python3 scripts/bench/compare_engines.py [--quick] [--only text,speed,...]

Needs: a release build (`cargo build --release -p akkhor-cli`), the big PDF
(`python3 scripts/make_fixtures.py --big 2000 --image-kb 150`), and
`pip install pymupdf pypdfium2` plus poppler-utils. Cold-cache numbers need
permission to write /proc/sys/vm/drop_caches; without it they are skipped.

Method (also written into the data's README):
- every measurement runs in a fresh process; library timings are taken
  inside the process around the library call;
- Poppler is driven through its command-line tools, so its timings include
  process start-up; the start-up of `pdfinfo -v` is measured alongside and
  subtracted (the raw numbers are kept too);
- speed: 5 runs, best and median reported; memory: resident memory added
  between just before opening and the peak after rendering.
"""

import argparse
import csv
import datetime
import json
import os
import platform
import random
import re
import statistics
import subprocess
import sys
import tempfile
import unicodedata

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
DATA = os.path.join(ROOT, "docs", "case-study", "data")
CORPUS = os.path.join(ROOT, "tests", "corpus")
BENCH = os.path.join(ROOT, "target", "release", "akkhor-bench")
ENGINES_PY = os.path.join(ROOT, "scripts", "bench", "engines.py")
BIG = os.path.join(ROOT, "tests", "fixtures", "big.pdf")

# Engine id -> display name; Akkhor twice: with only its bundled fonts, and
# with the corpus's other fonts registered, as the app registers the
# phone's fonts.
ENGINES = {
    "akkhor": "Akkhor",
    "akkhor+fonts": "Akkhor (+ device fonts)",
    "mupdf": "MuPDF",
    "pdfium": "PDFium",
    "poppler": "Poppler",
}


def call(engine, *args, timeout=600):
    """Runs one measurement in a fresh process; returns (json or None, returncode)."""
    env = dict(os.environ)
    if engine.startswith("akkhor"):
        cmd = [BENCH, *args]
        if engine == "akkhor+fonts":
            env["AKKHOR_REFERENCE_FONTS"] = os.path.join(ROOT, "tests", "fonts")
    else:
        cmd = [sys.executable, ENGINES_PY, engine, *args]
    try:
        out = subprocess.run(cmd, capture_output=True, timeout=timeout, env=env)
    except subprocess.TimeoutExpired:
        return None, "timeout"
    line = out.stdout.decode("utf-8", "replace").strip().splitlines()
    try:
        return json.loads(line[-1]) if line else None, out.returncode
    except json.JSONDecodeError:
        return None, out.returncode


# --- text accuracy ---------------------------------------------------------

def normal(text):
    """What a reader compares: canonical decomposition, no whitespace, no
    zero-width joiners."""
    text = unicodedata.normalize("NFD", text)
    return "".join(c for c in text if not c.isspace() and c not in "‌‍﻿­")


def edit_distance(a, b):
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j - 1] + (ca != cb), prev[j] + 1, cur[j - 1] + 1))
        prev = cur
    return prev[-1]


def words(text):
    return sorted({unicodedata.normalize("NFD", w) for w in re.split(r"[\s,।॥:;()\-!?\"“”]+", text) if len(w) >= 2})


def corpus_files():
    """(file, truth, language, set, producer, variant) for every corpus PDF."""
    rows = []
    for stem in ["bn", "hi", "bn-conjuncts", "hi-conjuncts"]:
        lang = "Bangla" if stem.startswith("bn") else "Hindi"
        design = "hind" if lang == "Bangla" else "mukta"
        for base, fonts in [(stem, "Noto Serif"), (f"{stem}.{design}", "Hind Siliguri" if lang == "Bangla" else "Mukta")]:
            for producer in ["chrome", "libreoffice"]:
                for variant in ["", ".raw"]:
                    rows.append((f"pdf/{base}.{producer}{variant}.pdf", f"{stem}.txt", lang,
                                 "conjunct list" if "conjuncts" in stem else "prose",
                                 fonts, "Chrome" if producer == "chrome" else "LibreOffice",
                                 "no ToUnicode" if variant else "as produced"))
    for producer in ["chrome", "libreoffice"]:
        for variant in ["", ".raw"]:
            rows.append((f"pdf/bn.bijoy.{producer}{variant}.pdf", "bn.txt", "Bangla (Bijoy)", "prose",
                         "SutonnyMJ (Bijoy)", "Chrome" if producer == "chrome" else "LibreOffice",
                         "no ToUnicode" if variant else "as produced"))
    return rows


def text_accuracy(engines):
    rows, samples = [], {}
    for file, truth_name, lang, kind, fonts, producer, variant in corpus_files():
        truth = open(os.path.join(CORPUS, truth_name), encoding="utf-8").read()
        t_norm = normal(truth)
        t_words = words(truth)
        for e in engines:
            out, code = call(e, "text", os.path.join(CORPUS, file))
            text = "\n".join(out["pages"]) if out and "pages" in out else ""
            g = normal(text)
            cer = edit_distance(t_norm, g) / max(1, len(t_norm))
            spaced = " ".join(unicodedata.normalize("NFD", text).split())
            found = sum(1 for w in t_words if w in spaced)
            rows.append({
                "engine": ENGINES[e], "file": file, "language": lang, "text": kind, "font": fonts,
                "producer": producer, "text_layer": variant, "cer": round(cer, 5),
                "words_found": round(found / max(1, len(t_words)), 5), "ms": round(out.get("ms", 0), 3) if out else "",
            })
            if file in ("pdf/bn.chrome.pdf", "pdf/hi.chrome.pdf", "pdf/bn.libreoffice.pdf",
                        "pdf/hi.libreoffice.pdf", "pdf/bn.bijoy.chrome.pdf"):
                first = next((l.strip() for l in text.splitlines() if l.strip()), "")
                samples.setdefault(file, {"truth": truth.splitlines()[0]})[ENGINES[e]] = first
            print(f"  {ENGINES[e]:<24} {file:<44} CER {cer * 100:6.1f}%  words {found}/{len(t_words)}", flush=True)
    return rows, samples


# --- speed and memory ------------------------------------------------------

def drop_caches():
    try:
        subprocess.run(["sync"])
        with open("/proc/sys/vm/drop_caches", "w") as f:
            f.write("3")
        return True
    except OSError:
        return False


def speed(engines, runs):
    cases = [
        (BIG, "big.pdf (2,000 pages, 308 MB)", 0, 1.0),
        (BIG, "big.pdf (2,000 pages, 308 MB)", 1999, 1.0),
        (BIG, "big.pdf (2,000 pages, 308 MB)", 1999, 2.0),
        (os.path.join(ROOT, "tests", "fixtures", "indic.pdf"), "indic.pdf (Bangla and Hindi page)", 0, 2.0),
        (os.path.join(CORPUS, "pdf", "bn.chrome.pdf"), "bn.chrome.pdf (Bangla prose)", 0, 2.0),
    ]
    rows = []
    for path, label, page, scale in cases:
        if not os.path.exists(path):
            print(f"  skipping {path} (generate it first)")
            continue
        for e in engines:
            out, _ = call(e, "speed", path, str(page), str(scale), str(runs))
            if not out or "open_ms" not in out:
                continue
            startup = statistics.median(out["startup_ms"]) if "startup_ms" in out else 0.0
            for metric in ("open_ms", "first_render_ms", "warm_render_ms"):
                vals = out[metric]
                adj = [max(0.0, v - startup) for v in vals]
                rows.append({
                    "engine": ENGINES[e], "file": label, "page": page + 1, "scale": scale,
                    "metric": metric.replace("_ms", ""), "best_ms": round(min(adj), 3),
                    "median_ms": round(statistics.median(adj), 3),
                    "raw_median_ms": round(statistics.median(vals), 3),
                    "startup_subtracted_ms": round(startup, 3),
                })
            print(f"  {ENGINES[e]:<24} {label:<36} p{page + 1} x{scale}: open {min(out['open_ms']):.2f} ms, "
                  f"render {min(out['first_render_ms']):.2f} ms (raw)", flush=True)
    # Cold open: the file not in the OS cache.
    cold = []
    if os.path.exists(BIG) and drop_caches():
        # Poppler runs as a new process, whose own start-up is also cold
        # after the caches are dropped; its cold numbers would measure that.
        for e in [e for e in engines if e != "poppler"]:
            vals = []
            for _ in range(3):
                drop_caches()
                out, _ = call(e, "speed", BIG, "1999", "1.0", "1")
                if out and "open_ms" in out:
                    startup = out.get("startup_ms", [0])[0]
                    vals.append((max(0, out["open_ms"][0] - startup), max(0, out["first_render_ms"][0] - startup)))
            if vals:
                cold.append({"engine": ENGINES[e], "file": "big.pdf (2,000 pages, 308 MB)",
                             "cold_open_ms": round(statistics.median(v[0] for v in vals), 2),
                             "cold_open_plus_last_page_ms": round(statistics.median(v[0] + v[1] for v in vals), 2)})
                print(f"  {ENGINES[e]:<24} cold open {cold[-1]['cold_open_ms']} ms", flush=True)
    return rows, cold


def memory(engines):
    cases = [(BIG, "big.pdf, last page at 1x", 1999, 1.0),
             (os.path.join(ROOT, "tests", "fixtures", "indic.pdf"), "indic.pdf at 2x", 0, 2.0)]
    rows = []
    for path, label, page, scale in cases:
        if not os.path.exists(path):
            continue
        for e in engines:
            vals = []
            for _ in range(3):
                out, _ = call(e, "mem", path, str(page), str(scale))
                if out and "peak_kb" in out:
                    vals.append(max(0, out["peak_kb"] - out["rss_before_kb"]))
            if vals:
                rows.append({"engine": ENGINES[e], "case": label, "added_mb": round(statistics.median(vals) / 1024, 2)})
                print(f"  {ENGINES[e]:<24} {label:<28} +{rows[-1]['added_mb']} MB", flush=True)
    return rows


# --- size ------------------------------------------------------------------

def stripped_size(path):
    with tempfile.TemporaryDirectory() as d:
        copy = os.path.join(d, "lib.so")
        subprocess.run(["cp", path, copy])
        subprocess.run(["strip", "--strip-unneeded", copy], capture_output=True)
        return os.path.getsize(copy)


def sizes():
    import pymupdf
    import pypdfium2_raw
    mu_dir = os.path.dirname(pymupdf.__file__)
    candidates = [
        ("Akkhor", os.path.join(ROOT, "target", "release", "libakkhor.so"),
         "PDF, EPUB, TXT, Indic shaping and text recovery"),
        ("MuPDF", next((os.path.join(mu_dir, f) for f in os.listdir(mu_dir) if f.startswith("libmupdf.so")), None),
         "PDF, XPS, EPUB, CBZ, ... (PyMuPDF wheel; fonts included)"),
        ("PDFium", os.path.join(os.path.dirname(pypdfium2_raw.__file__), "libpdfium.so"), "PDF only"),
        ("Poppler", "/lib/x86_64-linux-gnu/libpoppler.so.134", "PDF only; needs FreeType, cairo or Splash, fontconfig"),
    ]
    rows = []
    for name, path, scope in candidates:
        if path and os.path.exists(path):
            real = os.path.realpath(path)
            rows.append({"engine": name, "file": os.path.basename(real), "bytes": stripped_size(real),
                         "scope": scope, "platform": "x86_64 Linux, stripped"})
            print(f"  {name:<10} {rows[-1]['bytes'] / 1e6:6.2f} MB  {os.path.basename(real)}", flush=True)
    return rows


# --- robustness ------------------------------------------------------------

def mutate(data, rng):
    data = bytearray(data)
    kind = rng.randrange(4)
    n = len(data)
    if kind == 0:  # flip bytes
        for _ in range(rng.randrange(1, 40)):
            data[rng.randrange(n)] = rng.randrange(256)
    elif kind == 1:  # truncate
        data = data[: rng.randrange(n)]
    elif kind == 2:  # duplicate a block
        a = rng.randrange(n)
        b = min(n, a + rng.randrange(1, 4096))
        data[a:a] = data[a:b]
    else:  # zero a block
        a = rng.randrange(n)
        for i in range(a, min(n, a + rng.randrange(1, 2048))):
            data[i] = 0
    return bytes(data)


def robustness(engines, count):
    rng = random.Random(20261125)
    sources = [os.path.join(ROOT, "tests", "fixtures", "indic.pdf"),
               os.path.join(ROOT, "tests", "fixtures", "small.pdf"),
               os.path.join(CORPUS, "pdf", "hi.libreoffice.pdf"),
               os.path.join(CORPUS, "pdf", "bn.bijoy.chrome.pdf")]
    tallies = {e: {"opened": 0, "refused": 0, "crashed": 0, "hung": 0} for e in engines}
    with tempfile.TemporaryDirectory() as d:
        files = []
        for i in range(count):
            src = open(sources[i % len(sources)], "rb").read()
            path = os.path.join(d, f"damaged-{i}.pdf")
            open(path, "wb").write(mutate(src, rng))
            files.append(path)
        for e in engines:
            for path in files:
                out, code = call(e, "survive", path, timeout=30)
                t = tallies[e]
                if code == "timeout" or (out and out.get("hang")):
                    t["hung"] += 1
                elif out is None or (isinstance(code, int) and code < 0) or (out and out.get("crash")):
                    t["crashed"] += 1
                elif out.get("ok"):
                    t["opened"] += 1
                else:
                    t["refused"] += 1
            print(f"  {ENGINES[e]:<24} {tallies[e]}", flush=True)
    return [{"engine": ENGINES[e], "files": count, **t} for e, t in tallies.items()]


# --- environment -----------------------------------------------------------

def environment():
    def cmd(args):
        try:
            return subprocess.run(args, capture_output=True, text=True).stdout.strip().splitlines()[0]
        except Exception:
            return ""
    import pymupdf
    import pypdfium2
    cpu = next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name")), "")
    return {
        "date": datetime.date.today().isoformat(),
        "akkhor_commit": cmd(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"]),
        "rustc": cmd(["rustc", "--version"]),
        "cpu": cpu, "cores": os.cpu_count(), "os": platform.platform(),
        "python": platform.python_version(),
        "mupdf": pymupdf.VersionFitz, "pymupdf": pymupdf.VersionBind,
        "pdfium": str(getattr(pypdfium2, "PDFIUM_INFO", "")),
        "pypdfium2": getattr(pypdfium2, "__version__", str(getattr(pypdfium2, "V_PYPDFIUM2", ""))),
        "poppler": cmd(["/usr/bin/pdftoppm", "-v"]) or subprocess.run(["/usr/bin/pdftoppm", "-v"], capture_output=True, text=True).stderr.splitlines()[0],
    }


def write_csv(name, rows):
    if not rows:
        return
    with open(os.path.join(DATA, name), "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="text,speed,memory,size,robustness")
    ap.add_argument("--quick", action="store_true", help="fewer runs and damaged files")
    args = ap.parse_args()
    parts = set(args.only.split(","))
    os.makedirs(DATA, exist_ok=True)
    engines = list(ENGINES)
    perf_engines = ["akkhor", "mupdf", "pdfium", "poppler"]

    env = environment()
    json.dump(env, open(os.path.join(DATA, "environment.json"), "w"), indent=2, ensure_ascii=False)
    print("environment:", env)
    if "text" in parts:
        print("text accuracy")
        rows, samples = text_accuracy(engines)
        write_csv("text_accuracy.csv", rows)
        json.dump(samples, open(os.path.join(DATA, "samples.json"), "w", encoding="utf-8"), indent=2, ensure_ascii=False)
    if "speed" in parts:
        print("speed")
        rows, cold = speed(perf_engines, 2 if args.quick else 5)
        write_csv("speed.csv", rows)
        write_csv("cold_open.csv", cold)
    if "memory" in parts:
        print("memory")
        write_csv("memory.csv", memory(perf_engines))
    if "size" in parts:
        print("size")
        write_csv("size.csv", sizes())
    if "robustness" in parts:
        print("robustness")
        write_csv("robustness.csv", robustness(perf_engines, 40 if args.quick else 200))


if __name__ == "__main__":
    main()
