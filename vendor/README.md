# Vendored crates

## hayro-syntax 0.9.0

From [crates.io](https://crates.io/crates/hayro-syntax) (Laurenz Stampfl,
MIT OR Apache-2.0, license files kept in the folder), used through
`[patch.crates-io]` in the workspace `Cargo.toml`.

**Why:** upstream builds the whole page list when a PDF is opened, reading
the dictionary of every page. On a 2,000-page, 308 MB file that is 2,000
reads scattered over the file: 333 ms cold on CI, and 298 MB of the file
touched in memory.

**Change** (`src/page.rs` only; full diff in
`hayro-syntax.page-tree.diff`):

- `Pages` is lazy: the count comes from the root's `/Count`; page N is
  found by walking down the tree, using each node's `/Count` to skip whole
  subtrees (a flat node whose `/Count` equals its number of kids needs one
  read); each page is resolved once and kept.
- If the tree contradicts its own counts, it falls back to upstream's full
  walk, then to upstream's brute-force search.
- `Pages` no longer derefs to a slice; it has `len`, `is_empty`, `get`
  and `iter`, which is all its users need.
- `CachedPages::get` shortens the lifetime by hand (documented `unsafe`),
  because `Pages` became invariant once it held lazily filled cells.

To update: copy the new upstream version here, re-apply the diff, run the
workspace tests and `scripts/compare_renders.sh` (identical page counts and
renders on every corpus and fixture PDF). The change is meant to be
offered upstream.
