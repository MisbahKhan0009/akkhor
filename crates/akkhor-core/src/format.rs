/// Book formats the engine knows about. Whether a format can actually be
/// opened depends on which Cargo features the build enables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Pdf,
    Epub,
    /// MOBI, AZW, AZW3 (KF8) and PalmDOC: all Palm database containers.
    Mobi,
    Fb2,
    Cbz,
    Txt,
}

/// Detects the format from the first bytes of a file, reading at most a few
/// kilobytes. `name` (a file name or extension) is only used to tell zip
/// based formats apart when the content is ambiguous.
pub fn sniff(bytes: &[u8], name: Option<&str>) -> Option<Format> {
    let head = &bytes[..bytes.len().min(4096)];
    let ext = name
        .and_then(|n| n.rsplit('.').next())
        .map(|e| e.to_ascii_lowercase());

    // PDF: the spec allows junk before the header, so search the first KB.
    if find(&head[..head.len().min(1024)], b"%PDF-").is_some() {
        return Some(Format::Pdf);
    }

    // Zip containers. An EPUB stores "mimetype" uncompressed as its first entry.
    if head.starts_with(b"PK\x03\x04") {
        if find(head, b"application/epub+zip").is_some() {
            return Some(Format::Epub);
        }
        return match ext.as_deref() {
            Some("epub") => Some(Format::Epub),
            Some("fb2") | Some("zip") if find(head, b".fb2").is_some() => Some(Format::Fb2),
            _ => Some(Format::Cbz),
        };
    }

    // Palm database: the type/creator code sits at offset 60.
    if head.len() >= 68 {
        let code = &head[60..68];
        if code == b"BOOKMOBI" || code == b"TEXtREAd" {
            return Some(Format::Mobi);
        }
    }

    let probe = skip_bom(head);
    if (probe.starts_with(b"<?xml") || probe.starts_with(b"<FictionBook"))
        && find(probe, b"<FictionBook").is_some()
    {
        return Some(Format::Fb2);
    }

    if looks_like_text(head) {
        return Some(Format::Txt);
    }
    None
}

fn skip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// UTF-8 or UTF-16 text without binary control bytes. Legacy 8-bit encodings
/// (Bijoy-era Bangla text, Windows-1252) also pass: they have no NULs.
fn looks_like_text(b: &[u8]) -> bool {
    if b.is_empty() {
        return false;
    }
    if b.starts_with(b"\xFF\xFE") || b.starts_with(b"\xFE\xFF") {
        return true;
    }
    // UTF-16 without a byte-order mark: zeros in every other byte.
    let pairs = b.len() / 2;
    if pairs >= 4 {
        let odd = b.iter().skip(1).step_by(2).filter(|&&x| x == 0).count();
        let even = b.iter().step_by(2).filter(|&&x| x == 0).count();
        if (odd * 10 > pairs * 7 && even == 0) || (even * 10 > pairs * 7 && odd == 0) {
            return true;
        }
    }
    !b.iter()
        .any(|&c| c == 0 || (c < 0x09) || (0x0E..0x1B).contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats() {
        assert_eq!(sniff(b"%PDF-1.7\n...", None), Some(Format::Pdf));
        let mut epub = b"PK\x03\x04".to_vec();
        epub.extend_from_slice(&[0; 26]);
        epub.extend_from_slice(b"mimetypeapplication/epub+zip");
        assert_eq!(sniff(&epub, None), Some(Format::Epub));
        assert_eq!(
            sniff(b"PK\x03\x04 images", Some("x.cbz")),
            Some(Format::Cbz)
        );

        let mut mobi = vec![b' '; 60];
        mobi.extend_from_slice(b"BOOKMOBI");
        assert_eq!(sniff(&mobi, None), Some(Format::Mobi));

        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><FictionBook>", None),
            Some(Format::Fb2)
        );
        assert_eq!(sniff("আমার সোনার বাংলা".as_bytes(), None), Some(Format::Txt));
        assert_eq!(sniff(b"\x00\x01\x02", None), None);
    }
}
