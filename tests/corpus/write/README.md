# Copying pages: stress files

Made by `scripts/make_write_corpus.py`; used by
`crates/akkhor-pdf/tests/write.rs` and
`scripts/bench/check_page_copies.py`.

| File | What it tests |
|---|---|
| `inherited.pdf` | page size, fonts and rotation set on the page tree, two levels deep |
| `objstreams.pdf` | a corpus book with its objects in object streams and an xref stream |
| `annots.pdf` | a highlight, a sticky note, a free-text note, a link (dropped) and a form field whose type and name sit on a parent field |
| `encrypted.pdf` | AES-256 with an empty user password: opens, but a copy is refused |
