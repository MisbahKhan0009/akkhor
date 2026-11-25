# Test fonts

Extra fonts for the PDF accuracy corpus, so text recovery is tested on
designs other than the bundled Noto Serif ones. Both come from
[google/fonts](https://github.com/google/fonts) under the SIL Open Font
License 1.1 (see OFL.txt).

| File | Script | Designer |
| --- | --- | --- |
| HindSiliguri-Regular.ttf | Bangla | Indian Type Foundry |
| Mukta-Regular.ttf | Devanagari | Ek Type |

Neither keeps `uniXXXX` glyph names, so they exercise recovery by shaping
alone.
