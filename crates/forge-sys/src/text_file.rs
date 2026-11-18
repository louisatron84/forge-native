//! Reading and writing files for the editor: detects the encoding (byte
//! order marks, else UTF-8 if valid, else Western/Latin-1), spots binary
//! files, and refuses files too big to open. Replaces `src/main/textFile.ts`.

use forge_text::encoding::{Encoding, HUGE_FILE_BYTES, MAX_FILE_BYTES};
use std::io;
use std::path::Path;

/// What reading a file for the editor gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextFile {
    Text {
        text: String,
        encoding: Encoding,
        size: u64,
    },
    /// Has NUL bytes: probably not text. Read with `force` to open it anyway.
    Binary {
        size: u64,
    },
    /// Big enough to ask first. Read with `force` to open it anyway.
    Huge {
        size: u64,
    },
    /// Too big to open at all.
    TooLarge {
        size: u64,
    },
}

/// NUL bytes near the start mean it's almost certainly not a text file.
pub fn looks_binary(buf: &[u8]) -> bool {
    buf.iter().take(8000).any(|&b| b == 0)
}

fn bom_encoding(buf: &[u8]) -> Option<Encoding> {
    if buf.starts_with(&[0xef, 0xbb, 0xbf]) {
        Some(Encoding::Utf8bom)
    } else if buf.starts_with(&[0xff, 0xfe]) {
        Some(Encoding::Utf16le)
    } else if buf.starts_with(&[0xfe, 0xff]) {
        Some(Encoding::Utf16be)
    } else {
        None
    }
}

fn decode_utf16(buf: &[u8], little: bool) -> String {
    let units =
        buf.as_chunks::<2>().0.iter().map(|p| if little { u16::from_le_bytes(*p) } else { u16::from_be_bytes(*p) });
    char::decode_utf16(units).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

/// Decode with a known encoding (a byte order mark at the start is skipped).
pub fn decode_as(buf: &[u8], encoding: Encoding) -> String {
    let bom = bom_encoding(buf);
    match encoding {
        Encoding::Utf8 | Encoding::Utf8bom => {
            let body = if bom == Some(Encoding::Utf8bom) { &buf[3..] } else { buf };
            String::from_utf8_lossy(body).into_owned()
        }
        Encoding::Utf16le => decode_utf16(if bom == Some(Encoding::Utf16le) { &buf[2..] } else { buf }, true),
        Encoding::Utf16be => decode_utf16(if bom == Some(Encoding::Utf16be) { &buf[2..] } else { buf }, false),
        Encoding::Latin1 => buf.iter().map(|&b| b as char).collect(),
    }
}

/// Guess the encoding and decode.
pub fn decode(buf: &[u8]) -> (String, Encoding) {
    if let Some(bom) = bom_encoding(buf) {
        return (decode_as(buf, bom), bom);
    }
    match std::str::from_utf8(buf) {
        Ok(text) => (text.to_owned(), Encoding::Utf8),
        // Not valid UTF-8: most likely an old Windows/Western file.
        Err(_) => (decode_as(buf, Encoding::Latin1), Encoding::Latin1),
    }
}

pub fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8bom => [&[0xef, 0xbb, 0xbf][..], text.as_bytes()].concat(),
        Encoding::Utf16le => {
            let mut out = vec![0xff, 0xfe];
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        Encoding::Utf16be => {
            let mut out = vec![0xfe, 0xff];
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            out
        }
        // Characters Latin-1 can't hold are written as '?'.
        Encoding::Latin1 => text.chars().map(|c| u8::try_from(c as u32).unwrap_or(b'?')).collect(),
    }
}

/// Read a file for the editor. `force` opens binary and huge files anyway;
/// `encoding` reopens with a chosen encoding.
pub fn read_text_file(path: &Path, force: bool, encoding: Option<Encoding>) -> io::Result<TextFile> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_FILE_BYTES {
        return Ok(TextFile::TooLarge { size });
    }
    if size > HUGE_FILE_BYTES && !force {
        return Ok(TextFile::Huge { size });
    }
    let buf = std::fs::read(path)?;
    let bom = bom_encoding(&buf);
    // UTF-16 text is full of NUL bytes, so only check files without a UTF-16 byte order mark.
    let utf16 = matches!(bom, Some(Encoding::Utf16le | Encoding::Utf16be));
    if !force && encoding.is_none() && !utf16 && looks_binary(&buf) {
        return Ok(TextFile::Binary { size });
    }
    if let Some(enc) = encoding {
        return Ok(TextFile::Text { text: decode_as(&buf, enc), encoding: enc, size });
    }
    let (text, encoding) = decode(&buf);
    Ok(TextFile::Text { text, encoding, size })
}

/// Write a file in an encoding. Written to a temporary file next to it and
/// then moved into place, so a crash mid-save never leaves half a file.
pub fn write_text_file(path: &Path, text: &str, encoding: Encoding) -> io::Result<()> {
    crate::file_write::replace(path, encode(text, encoding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn failed_atomic_write_preserves_previous_contents_and_removes_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "previous").unwrap();
        let result = crate::file_write::replace_with(&path, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("simulated disk error"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn saving_never_overwrites_a_preexisting_temporary_path() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("private.txt");
        std::fs::write(&victim, "keep private data").unwrap();
        let trap = dir.path().join(format!(".main.py.forge-save-{}", std::process::id()));
        std::fs::hard_link(&victim, &trap).unwrap();
        let target = dir.path().join("main.py");
        write_text_file(&target, "print('hello')", Encoding::Utf8).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep private data");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "print('hello')");
        assert_eq!(std::fs::read_to_string(&trap).unwrap(), "keep private data");
    }

    #[test]
    fn reads_plain_utf8() {
        assert_eq!(decode("héllo ✓".as_bytes()), ("héllo ✓".to_owned(), Encoding::Utf8));
    }

    #[test]
    fn falls_back_to_latin1_for_bytes_that_are_not_valid_utf8() {
        assert_eq!(decode(&[0x63, 0x61, 0x66, 0xe9]), ("café".to_owned(), Encoding::Latin1));
    }

    #[test]
    fn recognises_byte_order_marks() {
        assert_eq!(decode(&[0xef, 0xbb, 0xbf, 0x61]), ("a".to_owned(), Encoding::Utf8bom));
        assert_eq!(decode(&[0xff, 0xfe, 0x61, 0x00]), ("a".to_owned(), Encoding::Utf16le));
        assert_eq!(decode(&[0xfe, 0xff, 0x00, 0x61]), ("a".to_owned(), Encoding::Utf16be));
    }

    #[test]
    fn round_trips_every_encoding() {
        for enc in Encoding::ALL {
            let text = if enc == Encoding::Latin1 { "café\r\nx" } else { "café ✓\r\nx" };
            assert_eq!(decode(&encode(text, enc)), (text.to_owned(), enc), "{enc:?}");
        }
    }

    #[test]
    fn spots_binary_data() {
        assert!(looks_binary(&[0x89, 0x50, 0x4e, 0x47, 0x00, 0x01]));
        assert!(!looks_binary(b"print(\"hi\")"));
    }

    #[test]
    fn reads_and_writes_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let img = dir.path().join("img.png");
        std::fs::write(&img, [0x89, 0x50, 0x00, 0x00, 0x41]).expect("write");
        assert_eq!(read_text_file(&img, false, None).expect("read"), TextFile::Binary { size: 5 });
        assert!(matches!(read_text_file(&img, true, None).expect("read"), TextFile::Text { .. }));

        // UTF-16 files (which contain NUL bytes) open as text.
        let u16 = dir.path().join("u16.txt");
        std::fs::write(&u16, encode("hi", Encoding::Utf16le)).expect("write");
        assert!(
            matches!(read_text_file(&u16, false, None).expect("read"), TextFile::Text { ref text, encoding: Encoding::Utf16le, .. } if text == "hi")
        );

        // Reopen with a chosen encoding.
        let w = dir.path().join("w.txt");
        std::fs::write(&w, "café".as_bytes()).expect("write");
        assert!(
            matches!(read_text_file(&w, false, Some(Encoding::Latin1)).expect("read"), TextFile::Text { ref text, .. } if text == "cafÃ©")
        );

        // Write in the requested encoding, creating folders.
        let out = dir.path().join("sub").join("out.txt");
        write_text_file(&out, "é", Encoding::Utf8bom).expect("write");
        assert_eq!(std::fs::read(&out).expect("read"), [0xef, 0xbb, 0xbf, 0xc3, 0xa9]);
        // No temporary files left behind.
        assert_eq!(std::fs::read_dir(dir.path().join("sub")).expect("dir").count(), 1);
    }
}
