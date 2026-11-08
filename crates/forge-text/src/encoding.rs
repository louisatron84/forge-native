//! Text encodings the editor can open and save files in, and the size limits
//! for opening files. Replaces `src/shared/encoding.ts` (decoding and
//! encoding themselves live with file reading in `forge-sys`).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Utf8,
    Utf8bom,
    Utf16le,
    Utf16be,
    Latin1,
}

impl Encoding {
    pub const ALL: [Encoding; 5] =
        [Encoding::Utf8, Encoding::Utf8bom, Encoding::Utf16le, Encoding::Utf16be, Encoding::Latin1];

    /// The id the old app used ("utf8", "utf8bom", ...).
    pub fn id(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf8",
            Encoding::Utf8bom => "utf8bom",
            Encoding::Utf16le => "utf16le",
            Encoding::Utf16be => "utf16be",
            Encoding::Latin1 => "latin1",
        }
    }

    /// Shown in the status bar and the encoding picker.
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8bom => "UTF-8 with BOM",
            Encoding::Utf16le => "UTF-16 LE",
            Encoding::Utf16be => "UTF-16 BE",
            Encoding::Latin1 => "Western (ISO 8859-1)",
        }
    }

    pub fn from_id(id: &str) -> Option<Encoding> {
        Encoding::ALL.into_iter().find(|e| e.id() == id)
    }
}

/// Files above this open in "large file mode": no language server, to stay responsive.
pub const LARGE_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Files above this ask before opening.
pub const HUGE_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Files above this aren't opened.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_labels_round_trip() {
        for e in Encoding::ALL {
            assert_eq!(Encoding::from_id(e.id()), Some(e));
        }
        assert_eq!(Encoding::Utf16le.label(), "UTF-16 LE");
        assert_eq!(serde_json::to_string(&Encoding::Utf8bom).expect("json"), "\"utf8bom\"");
    }
}
