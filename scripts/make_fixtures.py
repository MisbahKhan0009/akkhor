#!/usr/bin/env python3
"""Generates the test fixtures in tests/fixtures.

    python3 scripts/make_fixtures.py               # small fixtures (committed)
    python3 scripts/make_fixtures.py --big 2000    # also big.pdf with 2000 pages (not committed)

Only the Python standard library is used, so CI can run it anywhere.
"""

import argparse
import os
import zipfile
import zlib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tests", "fixtures")

BANGLA = [
    ("h1", "প্রথম অধ্যায়: অক্ষরের কথা"),
    ("p", "ভাষা আমাদের পরিচয়ের সবচেয়ে গভীর চিহ্ন। প্রতিটি যুক্তাক্ষর, প্রতিটি মাত্রা আর প্রতিটি হসন্ত মিলে তৈরি হয় এক একটি শব্দ, আর শব্দ মিলে তৈরি হয় আমাদের চিন্তার রূপ।"),
    ("p", "কিন্তু ডিজিটাল পর্দায় বাংলা পড়তে গেলে প্রায়ই দেখা যায়, ক্ষ ভেঙে ক্‌ষ হয়ে গেছে, ন্ত্র আলাদা হয়ে পড়েছে, কিংবা রেফ ভুল জায়গায় বসেছে। পাঠকের চোখে এই ভুলগুলো কাঁটার মতো বেঁধে।"),
    ("p", "অক্ষর ইঞ্জিনের লক্ষ্য খুব সহজ: হালকা, দ্রুত, আর নির্ভুল। স্বাস্থ্য, বিজ্ঞান, সাহিত্য, ইতিহাস কিংবা রাষ্ট্রবিজ্ঞান, যে বই-ই হোক, প্রতিটি যুক্তবর্ণ যেন <b>ঠিকঠাক</b> দেখা যায়।"),
    ("p", "উদাহরণ: ক্ষমা, মন্ত্রী, কর্ম, ব্যাঁক, উৎসব, স্নিগ্ধ, সন্ধ্যা, জ্যোৎস্না, উজ্জ্বল, বিদ্যুৎ, আত্মসম্মান, সংক্রান্ত, ক্ষমাপ্রার্থনাস্বরূপসংক্রান্তবিষয়াবলি।"),
]

HINDI = [
    ("h1", "दूसरा अध्याय: हिन्दी"),
    ("p", "भाषा केवल शब्दों का संग्रह नहीं है, वह एक संस्कृति की आत्मा है। देवनागरी लिपि में संयुक्ताक्षर जैसे क्ष, त्र, ज्ञ और श्र पढ़ने वाले के लिए स्वाभाविक हैं, पर कई सॉफ़्टवेयर इन्हें सही ढंग से नहीं दिखाते।"),
    ("p", "अक्षर इंजन का उद्देश्य है कि हर पुस्तक, चाहे वह विज्ञान की हो या साहित्य की, स्पष्ट और <i>सुंदर</i> दिखे। उदाहरण: शुद्ध, क्षत्रिय, श्रीमान, विद्यार्थी, राष्ट्र, स्वास्थ्य, द्वार, उज्ज्वल।"),
]

ENGLISH = [
    ("h1", "Chapter Three: Why Akkhor"),
    ("p", "Akkhor (অক্ষর, अक्षर) means letter. It is a small reading engine that keeps every conjunct whole, opens large files instantly and stays light on memory."),
    ("p", "This chapter mixes scripts on purpose: English, বাংলা and हिन्दी on one line should share a baseline and break cleanly."),
]


def xhtml(title, blocks, lang):
    body = "\n".join(f"<{t}>{x}</{t}>" for t, x in blocks)
    return f"""<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{lang}" xml:lang="{lang}">
<head><title>{title}</title><style>p {{ text-indent: 1em; }}</style></head>
<body>
{body}
</body>
</html>
"""


def make_epub(path):
    chapters = [("ch1.xhtml", "bn", BANGLA), ("ch2.xhtml", "hi", HINDI), ("ch3.xhtml", "en", ENGLISH)]
    container = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"""
    manifest = "\n".join(
        f'    <item id="c{i}" href="Text/{name}" media-type="application/xhtml+xml"/>'
        for i, (name, _, _) in enumerate(chapters)
    )
    spine = "\n".join(f'    <itemref idref="c{i}"/>' for i in range(len(chapters)))
    opf = f"""<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">akkhor-indic-sample</dc:identifier>
    <dc:title>অক্ষর: একটি নমুনা বই</dc:title>
    <dc:creator>Akkhor test suite</dc:creator>
    <dc:language>bn</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{manifest}
  </manifest>
  <spine toc="ncx">
{spine}
  </spine>
</package>
"""
    nav_items = "\n".join(
        f'      <li><a href="Text/{name}">{blocks[0][1]}</a></li>' for name, _, blocks in chapters
    )
    nav = f"""<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>সূচি</title></head>
<body><nav epub:type="toc"><ol>
{nav_items}
</ol></nav></body></html>
"""
    ncx_points = "\n".join(
        f'<navPoint id="n{i}" playOrder="{i+1}"><navLabel><text>{blocks[0][1]}</text></navLabel><content src="Text/{name}"/></navPoint>'
        for i, (name, _, blocks) in enumerate(chapters)
    )
    ncx = f"""<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>
{ncx_points}
</navMap></ncx>
"""
    with zipfile.ZipFile(path, "w") as z:
        z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml", container, compress_type=zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/content.opf", opf, compress_type=zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/nav.xhtml", nav, compress_type=zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/toc.ncx", ncx, compress_type=zipfile.ZIP_DEFLATED)
        for name, lang, blocks in chapters:
            z.writestr(f"OEBPS/Text/{name}", xhtml(blocks[0][1], blocks, lang), compress_type=zipfile.ZIP_DEFLATED)


def make_pdf(path, pages, image_kb=0):
    """A text-and-vector PDF with `pages` US Letter pages and compressed content.

    With `image_kb`, every page also carries an uncompressed photo-sized image
    of about that many KB, to make files of hundreds of MB.
    """
    objects = []

    def add(body):
        objects.append(body)
        return len(objects)

    font = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    pages_id = len(objects) + 1
    objects.append(None)  # placeholder for the page tree
    kids = []
    for n in range(1, pages + 1):
        lines = [b"BT /F1 24 Tf 72 720 Td (Akkhor stress test) Tj ET",
                 f"BT /F1 14 Tf 72 690 Td (Page {n} of {pages}) Tj ET".encode()]
        for i in range(30):
            y = 650 - i * 18
            lines.append(f"BT /F1 11 Tf 72 {y} Td (Line {i + 1}: The quick brown fox jumps over the lazy dog, page {n}.) Tj ET".encode())
        lines.append(f"0.{n % 9 + 1} 0.3 0.6 rg 420 40 120 60 re f".encode())
        lines.append(b"0 0 0 RG 2 w 72 60 m 300 100 l 520 60 l S")
        xobj = b""
        if image_kb:
            side = int((image_kb * 1024 / 3) ** 0.5)
            pixels = bytes((x * 7 + y * 3 + n) % 256 for y in range(side) for x in range(side) for _ in range(3))
            img = add(b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length %d >>\nstream\n"
                      % (side, side, len(pixels)) + pixels + b"\nendstream")
            lines.append(b"q 120 0 0 90 420 110 cm /Im1 Do Q")
            xobj = b" /XObject << /Im1 %d 0 R >>" % img
        data = zlib.compress(b"\n".join(lines))
        content = add(b"<< /Length %d /Filter /FlateDecode >>\nstream\n" % len(data) + data + b"\nendstream")
        page = add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >>%s >> /Contents %d 0 R >>"
                   % (pages_id, font, xobj, content))
        kids.append(page)
    objects[pages_id - 1] = b"<< /Type /Pages /Count %d /Kids [%s] >>" % (
        pages, b" ".join(b"%d 0 R" % k for k in kids))
    catalog = add(b"<< /Type /Catalog /Pages %d 0 R >>" % pages_id)

    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for i, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += b"trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, catalog, xref)
    with open(path, "wb") as f:
        f.write(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--big", type=int, default=0, help="also write big.pdf with this many pages")
    ap.add_argument("--image-kb", type=int, default=0, help="uncompressed image size per page in big.pdf")
    args = ap.parse_args()
    os.makedirs(ROOT, exist_ok=True)
    make_epub(os.path.join(ROOT, "indic-sample.epub"))
    make_pdf(os.path.join(ROOT, "small.pdf"), 3)
    if args.big:
        make_pdf(os.path.join(ROOT, "big.pdf"), args.big, args.image_kb)
    print("fixtures written to", os.path.normpath(ROOT))


if __name__ == "__main__":
    main()
