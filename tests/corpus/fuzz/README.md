# Inputs found by fuzzing

Each file here made an engine step crash, hang or take too long; it is
kept as a test so the fix stays: `*.pdf.bin` in
`crates/akkhor-pdf/tests/fuzz_found.rs`, `*.mobi.bin`, `*.fb2.bin` and
`*.cbz.bin` in `crates/akkhor/tests/fuzz_found.rs`. The files are
mutations of the test corpus made by `cargo fuzz`; `.bin` keeps them out
of the corpus-wide tests.

| File | Target | What happened | Fix |
|---|---|---|---|
| `nested-arrays-1.pdf.bin` | pdf | page text took 23 s | an unterminated nest of arrays fails at once instead of being re-read at every level (vendored hayro-syntax, lenient arrays) |
| `nested-arrays-2.pdf.bin` | pdf_write | page text did not finish | same |
| (in a unit test) | markup (CI) | a `%` before a multi-byte letter in a link or picture path panicked | percent codes decoded on bytes (`akkhor-epub`, `percent_decoding_keeps_letters_whole`) |
| `char-boundary-1.fb2.bin` | fb2 | opening panicked: the check for `<FictionBook` cut the text at byte 4096, inside a letter (any FB2 with a multi-byte letter there) | the first 4096 bytes are searched as bytes (`akkhor-fb2`) |
| (in a unit test) | sweep after the above | an XMP entity search cut text 12 bytes on, inside a letter (`&` before Bengali in a PDF's title) | searched as bytes (`akkhor-pdf`, `a_bare_ampersand_before_letters`) |
| `unclosed-heading-1.fb2.bin` | fb2 | a bold first paragraph never closed: the heading rewrite ran past the end | rewritten only when the paragraph closes |
| `huge-index-count-1.mobi.bin` | mobi | an index record claiming billions of entries: an 8 GB allocation, which aborts the app | the count is capped by the offset table the record holds |
| (in a unit test) | review of the above | a HUFF dictionary claiming 2^31 phrases (about 48 GB) | capped by its offsets (`a_dictionary_that_claims_billions_of_phrases`) |
