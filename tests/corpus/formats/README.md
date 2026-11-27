# Formats corpus

Made by `scripts/make_format_corpus.py` with Calibre 7.6 (an independent
writer), from the EPUB sources in `src/`: the same short book in Bangla,
Hindi and English, written for this corpus. Each has three chapters with
headings, paragraphs, a list, a table, a picture, a footnote, an
internal link, a cover and a table of contents.

| File | What it is |
|---|---|
| `src/*.epub` | the sources, and the truth: every other file must read the same |
| `*.kf7.mobi` | MOBI 7 (KF7), PalmDOC compression |
| `*.plain.mobi` | MOBI 7, no compression |
| `*.azw3` | KF8 only |
| `*.both.mobi` | combined KF7 + KF8 (the KF8 half is read) |
| `*.fb2` | FictionBook 2 |
