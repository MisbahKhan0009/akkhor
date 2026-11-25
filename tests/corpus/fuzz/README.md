# Inputs found by fuzzing

Each file here made an engine step crash, hang or take too long; it is
kept as a test (`crates/akkhor-pdf/tests/fuzz_found.rs`) so the fix stays.
The files are mutations of the test corpus made by `cargo fuzz`; `.bin`
keeps them out of the corpus-wide PDF tests.

| File | Target | What happened | Fix |
|---|---|---|---|
| `nested-arrays-1.pdf.bin` | pdf | page text took 23 s | an unterminated nest of arrays fails at once instead of being re-read at every level (vendored hayro-syntax, lenient arrays) |
| `nested-arrays-2.pdf.bin` | pdf_write | page text did not finish | same |
