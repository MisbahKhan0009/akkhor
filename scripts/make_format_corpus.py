#!/usr/bin/env python3
"""Makes tests/corpus/formats: the same three books (Bangla, Hindi,
English) as EPUB sources and as MOBI (KF7), AZW3 (KF8), combined MOBI
(KF7 + KF8) and FB2, written by Calibre (an independent implementation),
hand-written FB2 files (titles, nested sections, poems, notes,
pictures, Windows-1251, zipped), plus comics as CBZ.

  sudo apt-get install calibre      # 7.6 used for the committed files
  python3 scripts/make_format_corpus.py

The text is written for this corpus (no copyrighted text). Each book has
headings, paragraphs, a list, a table, a picture, an internal link, a
footnote, a cover and a table of contents, in three chapters.
"""
import io, os, subprocess, zipfile
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tests/corpus/formats")

BOOKS = {
    "bn": {
        "title": "নদীর ধারের গ্রাম",
        "author": "অক্ষর পরীক্ষা",
        "chapters": [
            ("প্রথম অধ্যায়: সকাল", [
                "ভোরবেলা নদীর ঘাটে কুয়াশা জমে থাকে। মাঝিরা নৌকা বাঁধে আর ছেলেমেয়েরা স্কুলের পথে দৌড়ায়।",
                "গ্রামের পশ্চিমে একটা পুরনো বটগাছ আছে, তার ছায়ায় দুপুরে সবাই বিশ্রাম নেয়।",
            ], ["ধান", "পাট", "সর্ষে"], [("ফসল", "মাস"), ("ধান", "অগ্রহায়ণ"), ("পাট", "শ্রাবণ")]),
            ("দ্বিতীয় অধ্যায়: হাট", [
                "সপ্তাহে দুদিন হাট বসে। দূরের গ্রাম থেকে লোকজন ক্ষেতের সবজি আর হাতের কাজ নিয়ে আসে।",
                "বিকেলে ঝড়ের আভাস দেখা দিলে দোকানিরা তাড়াতাড়ি জিনিসপত্র গুছিয়ে নেয়।",
            ], None, None),
            ("তৃতীয় অধ্যায়: সন্ধ্যা", [
                "সন্ধ্যায় উঠোনে প্রদীপ জ্বলে, বৃদ্ধরা পুরনো দিনের গল্প শোনান।",
                "রাত গভীর হলে শুধু ঝিঁঝিঁ পোকার ডাক আর নদীর স্রোতের শব্দ শোনা যায়।",
            ], None, None),
        ],
        "note": "বটগাছটির বয়স প্রায় দুশো বছর বলে লোকে বিশ্বাস করে।",
        "link": "প্রথম অধ্যায়ে ফিরে যান",
    },
    "hi": {
        "title": "पहाड़ों का गाँव",
        "author": "अक्षर परीक्षण",
        "chapters": [
            ("पहला अध्याय: सुबह", [
                "सुबह पहाड़ों पर धूप धीरे-धीरे उतरती है। बच्चे स्कूल के रास्ते में झरने के पास रुकते हैं।",
                "गाँव के बीच में एक पुराना मंदिर है, जहाँ शाम को सब इकट्ठा होते हैं।",
            ], ["गेहूँ", "मक्का", "सरसों"], [("फ़सल", "महीना"), ("गेहूँ", "चैत्र"), ("मक्का", "श्रावण")]),
            ("दूसरा अध्याय: मेला", [
                "हर साल वसंत में मेला लगता है। दूर-दूर से लोग हस्तशिल्प और मिठाइयाँ लेकर आते हैं।",
                "शाम को कठपुतली का खेल देखने के लिए भीड़ जमा हो जाती है।",
            ], None, None),
            ("तीसरा अध्याय: रात", [
                "रात में आसमान तारों से भर जाता है और ठंडी हवा चलती है।",
                "दादी कहानियाँ सुनाती हैं और बच्चे धीरे-धीरे सो जाते हैं।",
            ], None, None),
        ],
        "note": "मंदिर लगभग तीन सौ साल पुराना माना जाता है।",
        "link": "पहले अध्याय पर लौटें",
    },
    "en": {
        "title": "The River Village",
        "author": "Akkhor Test",
        "chapters": [
            ("Chapter One: Morning", [
                "Mist settles over the river landing at dawn. Boatmen tie up their boats while children run to school.",
                "An old banyan tree stands west of the village, and everyone rests in its shade at noon.",
            ], ["Rice", "Jute", "Mustard"], [("Crop", "Month"), ("Rice", "November"), ("Jute", "August")]),
            ("Chapter Two: Market", [
                "The market meets twice a week. People bring vegetables and handicrafts from distant villages.",
                "When a storm threatens in the afternoon, the sellers pack up their goods in a hurry.",
            ], None, None),
            ("Chapter Three: Evening", [
                "Lamps are lit in the courtyards at dusk, and the elders tell stories of the old days.",
                "Late at night only the crickets and the current of the river can be heard.",
            ], None, None),
        ],
        "note": "People believe the banyan tree is nearly two hundred years old.",
        "link": "Back to chapter one",
    },
}


def png(w, h, colour, label):
    img = Image.new("RGB", (w, h), colour)
    d = ImageDraw.Draw(img)
    d.rectangle([8, 8, w - 9, h - 9], outline=(255, 255, 255), width=4)
    d.text((20, h // 2 - 6), label, fill=(255, 255, 255))
    b = io.BytesIO()
    img.save(b, "PNG")
    return b.getvalue()


def chapter_html(lang, book, i, ch):
    title, paras, items, table = ch
    body = [f'<h1 id="c{i}">{title}</h1>']
    for k, p in enumerate(paras):
        if i == 0 and k == 1:
            body.append(f'<p>{p}<a href="notes.xhtml#n1" epub:type="noteref" id="r1"><sup>1</sup></a></p>')
        else:
            body.append(f"<p>{p}</p>")
    if items:
        body.append("<ul>" + "".join(f"<li>{x}</li>" for x in items) + "</ul>")
    if table:
        head, *rows = table
        body.append("<table><tr>" + "".join(f"<th>{c}</th>" for c in head) + "</tr>"
                    + "".join("<tr>" + "".join(f"<td>{c}</td>" for c in r) + "</tr>" for r in rows) + "</table>")
        body.append('<p><img src="images/field.png" alt="field"/></p>')
    if i == 2:
        body.append(f'<p><a href="ch0.xhtml#c0">{book["link"]}</a></p>')
    return (f'<?xml version="1.0" encoding="utf-8"?>\n<html xmlns="http://www.w3.org/1999/xhtml" '
            f'xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{lang}" lang="{lang}"><head><title>{title}</title>'
            f'<link rel="stylesheet" href="style.css"/></head><body>{"".join(body)}</body></html>')


def epub(lang, book, path):
    z = zipfile.ZipFile(path, "w")
    z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip")
    z.writestr("META-INF/container.xml", '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>', zipfile.ZIP_DEFLATED)
    n = len(book["chapters"])
    manifest = "".join(f'<item id="ch{i}" href="ch{i}.xhtml" media-type="application/xhtml+xml"/>' for i in range(n))
    manifest += ('<item id="notes" href="notes.xhtml" media-type="application/xhtml+xml"/>'
                 '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
                 '<item id="css" href="style.css" media-type="text/css"/>'
                 '<item id="cover" href="images/cover.png" media-type="image/png" properties="cover-image"/>'
                 '<item id="field" href="images/field.png" media-type="image/png"/>')
    spine = "".join(f'<itemref idref="ch{i}"/>' for i in range(n)) + '<itemref idref="notes"/>'
    opf = (f'<?xml version="1.0" encoding="utf-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">'
           f'<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">akkhor-{lang}</dc:identifier>'
           f'<dc:title>{book["title"]}</dc:title><dc:creator>{book["author"]}</dc:creator><dc:language>{lang}</dc:language>'
           f'<dc:publisher>Akkhor</dc:publisher><meta property="dcterms:modified">2026-11-27T00:00:00Z</meta><meta name="cover" content="cover"/></metadata>'
           f'<manifest>{manifest}</manifest><spine>{spine}</spine></package>')
    z.writestr("OEBPS/content.opf", opf, zipfile.ZIP_DEFLATED)
    for i, ch in enumerate(book["chapters"]):
        z.writestr(f"OEBPS/ch{i}.xhtml", chapter_html(lang, book, i, ch), zipfile.ZIP_DEFLATED)
    z.writestr("OEBPS/notes.xhtml", f'<?xml version="1.0" encoding="utf-8"?>\n<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{lang}"><head><title>notes</title></head><body><aside epub:type="footnote" id="n1"><p><a href="ch0.xhtml#r1">1</a> {book["note"]}</p></aside></body></html>', zipfile.ZIP_DEFLATED)
    nav = "".join(f'<li><a href="ch{i}.xhtml#c{i}">{c[0]}</a></li>' for i, c in enumerate(book["chapters"]))
    z.writestr("OEBPS/nav.xhtml", f'<?xml version="1.0" encoding="utf-8"?>\n<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>toc</title></head><body><nav epub:type="toc"><ol>{nav}</ol></nav></body></html>', zipfile.ZIP_DEFLATED)
    z.writestr("OEBPS/style.css", "h1{font-size:1.6em;text-align:center} p{text-indent:1.5em} table{border-collapse:collapse} td,th{border:1px solid #444;padding:4px}", zipfile.ZIP_DEFLATED)
    z.writestr("OEBPS/images/cover.png", png(600, 900, (47, 79, 74), book["title"].encode("ascii", "ignore").decode() or lang))
    z.writestr("OEBPS/images/field.png", png(400, 200, (107, 78, 30), "field"))
    z.close()


def convert(src, dst, *opts):
    subprocess.run(["ebook-convert", src, dst, *opts], check=True, capture_output=True)


FB2_RICH = """<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink">
<description><title-info><genre>poetry</genre>
<author><first-name>অক্ষর</first-name><last-name>পরীক্ষা</last-name></author>
<author><nickname>দ্বিতীয় লেখক</nickname></author>
<book-title>কবিতা ও গল্প</book-title>
<annotation><p>একটি ছোট পরীক্ষার বই।</p></annotation>
<coverpage><image l:href="#cover.png"/></coverpage><lang>bn</lang>
<sequence name="অক্ষর সিরিজ" number="2"/></title-info>
<publish-info><publisher>অক্ষর প্রকাশনী</publisher></publish-info></description>
<body><title><p>কবিতা ও গল্প</p></title>
<epigraph><p>শব্দই পথ দেখায়।</p><text-author>অজানা</text-author></epigraph>
<section id="s1"><title><p>প্রথম অংশ</p><p>সকালের গান</p></title>
<p>নদীর <emphasis>ধারে</emphasis> একটি <strong>ছোট</strong> গ্রাম।<a l:href="#n1" type="note">[1]</a></p>
<poem><title><p>ছড়া</p></title><stanza><v>আকাশ জুড়ে মেঘ,</v><v>বাতাসে ভেজা গন্ধ।</v></stanza></poem>
<empty-line/>
<image l:href="#pic.png"/>
<section><title><p>উপ-অংশ</p></title><p>ভেতরের অংশের লেখা।</p></section>
</section>
<section id="s2"><title><p>দ্বিতীয় অংশ</p></title><subtitle>* * *</subtitle>
<cite><p>উদ্ধৃতির লেখা।</p></cite>
<table><tr><th>ফসল</th><th>মাস</th></tr><tr><td>ধান</td><td>অগ্রহায়ণ</td></tr></table></section>
</body>
<body name="notes"><title><p>টীকা</p></title>
<section id="n1"><title><p>1</p></title><p>গ্রামটি নদীর পূর্ব পাড়ে।</p></section></body>
<binary id="cover.png" content-type="image/png">COVER</binary>
<binary id="pic.png" content-type="image/png">PIC</binary>
</FictionBook>
"""

FB2_1251 = """<?xml version="1.0" encoding="windows-1251"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
<description><title-info><author><first-name>Тест</first-name><last-name>Акхор</last-name></author>
<book-title>Деревня у реки</book-title><lang>ru</lang></title-info></description>
<body><section><title><p>Глава первая</p></title><p>Утром над рекой стоит туман. «Ёлка» — №1.</p></section>
<section><title><p>Глава вторая</p></title><p>Рынок собирается дважды в неделю.</p></section></body>
</FictionBook>
"""


def fb2_samples():
    import base64
    cover = base64.b64encode(png(300, 450, (62, 74, 92), "fb2")).decode()
    pic = base64.b64encode(png(200, 100, (58, 63, 44), "pic")).decode()
    wrapped = lambda s: "\n".join(s[i:i + 76] for i in range(0, len(s), 76))
    rich = FB2_RICH.replace("COVER", wrapped(cover)).replace("PIC", wrapped(pic))
    open(f"{OUT}/rich.bn.fb2", "w", encoding="utf-8").write(rich)
    with zipfile.ZipFile(f"{OUT}/rich.bn.fb2.zip", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("rich.bn.fb2", rich)
    open(f"{OUT}/cp1251.ru.fb2", "wb").write(FB2_1251.encode("cp1251"))


def main():
    os.makedirs(f"{OUT}/src", exist_ok=True)
    fb2_samples()
    for lang, book in BOOKS.items():
        src = f"{OUT}/src/{lang}.epub"
        epub(lang, book, src)
        convert(src, f"{OUT}/{lang}.kf7.mobi", "--mobi-file-type", "old")
        convert(src, f"{OUT}/{lang}.both.mobi", "--mobi-file-type", "both")
        convert(src, f"{OUT}/{lang}.azw3")
        convert(src, f"{OUT}/{lang}.fb2")
        # Uncompressed text records too (PalmDOC compression off).
        convert(src, f"{OUT}/{lang}.plain.mobi", "--mobi-file-type", "old", "--dont-compress")
        print(lang, "done")


if __name__ == "__main__":
    main()
