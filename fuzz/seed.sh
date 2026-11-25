#!/bin/sh
# Fills fuzz/corpus/<target> with starting inputs from the test corpus.
# Run from the repository root: sh fuzz/seed.sh
set -e
c=fuzz/corpus
mkdir -p $c/pdf $c/pdf_reflow $c/pdf_write $c/epub $c/zip $c/markup $c/text $c/locator
for f in tests/corpus/pdf/*.pdf tests/corpus/books/pdf/*.pdf tests/corpus/tables/pdf/*.pdf \
         tests/corpus/write/*.pdf tests/fixtures/small.pdf tests/fixtures/indic.pdf; do
    for t in pdf pdf_reflow pdf_write; do cp "$f" $c/$t/; done
done
cp tests/fixtures/indic-sample.epub $c/epub/
cp tests/fixtures/indic-sample.epub $c/zip/
# Chapters and style sheets of the sample book, as "markup NUL css".
python3 - <<'PY'
import zipfile
z = zipfile.ZipFile("tests/fixtures/indic-sample.epub")
css = b"".join(z.read(n) for n in z.namelist() if n.endswith(".css"))
for i, n in enumerate(n for n in z.namelist() if n.endswith((".xhtml", ".html"))):
    open(f"fuzz/corpus/markup/ch{i}", "wb").write(z.read(n) + b"\0" + css)
PY
for f in tests/corpus/*.txt tests/corpus/books/*.txt tests/fixtures/bijoy-sample.txt; do cp "$f" $c/text/; done
i=0
for l in "akkhor:1/c0/b0/o0" "akkhor:1/c12/b345/o6789" "akkhor:1/p0/o0" "akkhor:1/p41/o120"; do
    printf %s "$l" > $c/locator/l$i; i=$((i + 1))
done
