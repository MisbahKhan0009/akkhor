# LinkedIn post draft

*Images to attach, in this order (all in `figures/`):*

1. `copied-text-samples.png`: the hook, readable without any context
2. `reflow-showcase.png`: a two-column PDF page reflowed for a phone
3. `text-accuracy.png`
4. `books-words-intact.png` or `real-world-book.png`
5. `speed-big-pdf.png` (or `cold-open.png`)

---

Copy a line of Bangla out of a PDF and paste it into a search box. Very
often it doesn't come back whole. Some words get split by stray spaces,
some get garbled, and Bangla typed in the old Bijoy encoding comes back as
"evsjv fvlvi".

I've been building **Akkhor** (অক্ষর, "letter"), a small reading engine in
Rust, to fix exactly that, then to power an Android reader.

What it does differently:

🔹 It gets the letters back from the glyphs themselves. Each embedded glyph
is found by its outline in a complete font, and that font's shaping rules
are run backwards to find which syllable drew it. Then the text is put back
into reading order (ি, the reph, split vowels).

🔹 Bijoy text is converted to Unicode automatically, in PDFs and text files.

🔹 It reflows PDFs: a two-column book page becomes paragraphs on a phone
screen, without running headers or page numbers, in any theme.

🔹 It's built for phones: files are memory-mapped and only the pages you
open are read.

Measured against MuPDF, PDFium (the engine inside Chrome) and Poppler, on
36 test PDFs in Bangla and Hindi:

📄 PDFs with no text information: Akkhor 0% wrong characters when it knows
the font, the others 97 to 100%
🔤 Bijoy PDFs: Akkhor 0%, the others about 96%
🔎 Words you can find by search: Akkhor 100%, the others 86 to 97%
📚 A real 200-page Bangla book: 100% of words whole with its free fonts
registered (68% without), the others about 26%
⚡ A 2,000-page, 308 MB PDF: opens in 0.2 ms, last page drawn in 2.7 ms
(MuPDF 12.8 ms, PDFium 15.8 ms). Straight from disk, open + last page:
13.5 ms (MuPDF 219 ms, PDFium 185 ms)

It isn't ahead everywhere. MuPDF still redraws a page faster (1.6 ms vs
2.3 ms, down from 5.4) and uses a little less memory. And I missed my library size target: 5.8 MB, not 3.5. Those
are next on the list.
Every number, chart and script is in the repo, so anyone can rerun it.

Engine first, the Android reader next.

🔗 github.com/MisbahKhan0009/akkhor

#Rust #Bangla #Hindi #Unicode #PDF #OpenSource #Android #Typography

---

*Notes for editing:*

- The test PDFs were made for the project (Chrome and LibreOffice, four
  fonts). If someone asks, say so: real-world PDFs are the next test set.
- "0% when it knows the font": without the font (not bundled, not on the
  phone), a PDF with no text information can't be recovered. The
  `text-accuracy` chart shows both cases.
- Numbers come from `data/` (run of 2026-11-25). Rerun
  `scripts/bench/compare_engines.py` before posting if the code has changed.
