# Test fonts

Extra fonts for the PDF accuracy corpus, so text recovery is tested on
designs other than the bundled Noto Serif ones. All are under the SIL Open
Font License 1.1.

| File | Script | Designer | Source | Licence |
| --- | --- | --- | --- | --- |
| HindSiliguri-Regular.ttf | Bangla | Indian Type Foundry | [google/fonts](https://github.com/google/fonts) | OFL.txt |
| Mukta-Regular.ttf | Devanagari | Ek Type | [google/fonts](https://github.com/google/fonts) | OFL.txt |
| Kalpurush-Regular.ttf | Bangla | Md. Tanbin Islam Siyam (Avro font project) | [potasiyam/Kalpurush](https://github.com/potasiyam/Kalpurush), release 0.258 | Kalpurush-OFL.txt |

Kalpurush is the default font of the Avro keyboard and the most common
Bangla font in PDFs made in Bangladesh with Word; it is in the corpus for
that reason. None of the three keeps `uniXXXX` glyph names, so they
exercise recovery by shaping alone.
