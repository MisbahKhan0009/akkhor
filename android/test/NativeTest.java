// Calls every kind of engine function through JNI from a plain JVM, against
// the host build of libakkhor, with the declarations generated from
// android/Native.kt. A mismatched signature or a crash fails CI here, not
// in the app. Bitmaps exist only on Android: render calls must fail cleanly.
//
//   cargo build -p akkhor-android
//   python3 scripts/check_native.py --java /tmp/jni
//   javac -encoding UTF-8 -d /tmp/jni /tmp/jni/dev/akkhor/Native.java android/test/NativeTest.java
//   java -Djava.library.path=target/debug -cp /tmp/jni NativeTest

import dev.akkhor.Native;

public class NativeTest {
    static int failures = 0;

    static void check(boolean ok, String what) {
        if (!ok) {
            failures++;
            System.out.println("FAIL " + what + " (lastError: " + Native.lastError() + ")");
        }
    }

    static double field(String json, String key) {
        int i = json.indexOf("\"" + key + "\":");
        if (i < 0) return Double.NaN;
        int s = i + key.length() + 3, e = s;
        while (e < json.length() && "0123456789.-".indexOf(json.charAt(e)) >= 0) e++;
        return Double.parseDouble(json.substring(s, e));
    }

    public static void main(String[] args) throws Exception {
        String root = args.length > 0 ? args[0] : ".";
        check(Native.apiVersion() == 1, "apiVersion");

        // Errors are values, never crashes.
        check(Native.format(999_999) == -1, "format of a bad handle");
        check(Native.lastError() != null, "lastError after a failure");
        Native.close(999_999);
        check(Native.openPath(root + "/no/such/file.epub") == 0, "opening a missing file");
        check(Native.info(999_999) == null, "info of a bad handle");
        check(Native.clampLocator(999_999, "not a locator") == null, "a bad locator");

        Native.registerFont("Latn", root + "/fonts/NotoSerif-Regular.ttf");
        Native.registerFont("Beng", root + "/fonts/NotoSerifBengali-Regular.ttf");
        Native.registerFont("Deva", root + "/fonts/NotoSerifDevanagari-Regular.ttf");
        check(Native.setThemeColors(16, 0xFFF7F3EA, 0xFF221F1A) == 0, "an app theme");
        check(Native.setThemeColors(5, 0, 0) == -1, "built-in theme codes are not the app's");

        // EPUB: info, chapters, layout, locators, ranges, sentences, search.
        long h = Native.openPath(root + "/tests/fixtures/indic-sample.epub");
        check(h != 0, "open EPUB");
        check(Native.format(h) == 1, "EPUB format");
        String info = Native.info(h);
        check(info != null && info.contains("\"title\":\"অক্ষর: একটি নমুনা বই\""), "EPUB info: " + info);
        check(Native.chapterCount(h) == 3, "chapterCount");
        int screens = Native.layoutChapter(h, 0, 720, 900, 40f);
        check(screens > 1, "layoutChapter: " + screens);
        String second = Native.locatorOfScreen(h, 1);
        check(second != null && second.startsWith("akkhor:1/c0/"), "locatorOfScreen: " + second);
        check(Native.screenOfLocator(h, second) == 1, "screenOfLocator");
        check(Native.screenOfLocator(h, "akkhor:1/c2/b0/o0") == -1, "a place in another chapter");
        String start = "akkhor:1/c0/b0/o0";
        String text = Native.textOfRange(h, start, second);
        check(text != null && text.startsWith("প্রথম অধ্যায়"), "textOfRange: " + text);
        float[] rects = Native.rectsOfRange(h, 0, start, second);
        check(rects != null && rects.length > 0 && rects.length % 4 == 0, "rectsOfRange");
        String mid = Native.locatorAtProgress(h, 0.5);
        double p = Native.progressOf(h, mid);
        check(Math.abs(p - 0.5) < 0.01, "progress round trip: " + p);
        check(Native.clampLocator(h, "akkhor:1/c99/b0/o0").startsWith("akkhor:1/c2/"), "clamp past the end");
        String sentences = Native.sentences(h, start, 3);
        check(sentences != null && sentences.startsWith("[{\"start\":"), "sentences: " + sentences);

        long s = Native.searchStart(h, "অক্ষর");
        check(s != 0, "searchStart");
        int hits = 0;
        while (true) {
            String batch = Native.searchNext(s, 2);
            check(batch != null, "searchNext");
            if (batch == null) break;
            hits += batch.split("\"snippet\"").length - 1;
            if (batch.contains("\"done\":true")) break;
        }
        check(hits >= 2, "search hits: " + hits);
        check(Native.searchNext(s, 2) == null, "a finished search is gone");
        Native.searchCancel(s);

        // The same book from several threads at once.
        final long book = h;
        Thread[] ts = new Thread[6];
        final int[] bad = {0};
        for (int i = 0; i < ts.length; i++) {
            ts[i] = new Thread(() -> {
                for (int k = 0; k < 20; k++) {
                    if (Native.info(book) == null) bad[0]++;
                    long id = Native.searchStart(book, "শব্দ");
                    if (Native.searchNext(id, 5) == null) bad[0]++;
                    Native.searchCancel(id);
                    if (Native.progressOf(book, "akkhor:1/c1/b0/o0") < 0) bad[0]++;
                }
            });
            ts[i].start();
        }
        for (Thread t : ts) t.join();
        check(bad[0] == 0, "calls from several threads: " + bad[0] + " failed");
        Native.close(h);
        check(Native.info(h) == null, "info after close");
        Native.close(h);

        // PDF: info, places in both views, page text; drawing needs Android.
        long pdf = Native.openPath(root + "/tests/corpus/books/pdf/bn.chrome.two.pdf");
        check(pdf != 0 && Native.format(pdf) == 0, "open PDF");
        String pinfo = Native.info(pdf);
        check(pinfo != null && pinfo.contains("\"format\":\"pdf\"") && field(pinfo, "text_quality") >= 0.98,
              "PDF info: " + pinfo);
        String flow = Native.convertLocator(pdf, "akkhor:1/p1/o200");
        check(flow != null && flow.startsWith("akkhor:1/c"), "page place to reflow place: " + flow);
        check(Native.convertLocator(pdf, flow) != null, "and back");
        check(Native.pdfPageText(pdf, 0) != null, "pdfPageText");
        check(Native.renderPdfTile(pdf, 0, 1f, 0, 0, new Object(), 16) < 0, "drawing needs an Android bitmap");
        check(Native.renderCover(pdf, new Object()) == -1, "cover needs an Android bitmap");
        // Two pages, last first, as a new PDF that reads the same.
        String out = System.getProperty("java.io.tmpdir") + "/akkhor-copy.pdf";
        check(Native.pdfWritePages(pdf, new int[] {1, 0}, out) > 1000, "pdfWritePages");
        long copy = Native.openPath(out);
        check(copy != 0 && Native.format(copy) == 0, "open the copy");
        check(Native.pdfPageText(pdf, 1).equals(Native.pdfPageText(copy, 0)), "the copy's text");
        Native.close(copy);
        check(Native.pdfWritePages(pdf, new int[] {7}, out) == -1, "a page out of range");
        check(Native.pdfWritePages(pdf, new int[] {-1}, out) == -1, "a negative page");
        Native.close(pdf);
        long locked = Native.openPath(root + "/tests/corpus/write/encrypted.pdf");
        check(locked != 0, "an encrypted PDF with an empty password opens");
        check(Native.pdfWritePages(locked, new int[] {0}, out) == -1, "but is not copied");
        check(String.valueOf(Native.lastError()).contains("protected"), "lastError: " + Native.lastError());
        Native.close(locked);

        // Strings: UTF-8 bytes against UTF-16 units.
        check(Native.utf16Offset("আ😀b", 7) == 3, "utf16Offset");
        check(Native.utf8Offset("আ😀b", 3) == 7, "utf8Offset");

        if (failures > 0) {
            System.out.println(failures + " failures");
            System.exit(1);
        }
        System.out.println("NativeTest: all calls behaved");
    }
}
