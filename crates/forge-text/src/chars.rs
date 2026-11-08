//! Character-indexed string helpers. Editor columns count `char`s, while Rust
//! strings index bytes; these convert, with a fast path for ASCII lines.

/// Length in characters.
pub fn len(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.chars().count()
    }
}

/// Byte index of character `col` (the string's length past the end).
pub fn byte_at(s: &str, col: usize) -> usize {
    if s.is_ascii() {
        return col.min(s.len());
    }
    s.char_indices().nth(col).map_or(s.len(), |(i, _)| i)
}

/// Characters `from..to` (clamped).
pub fn slice(s: &str, from: usize, to: usize) -> &str {
    let a = byte_at(s, from);
    let b = byte_at(s, to.max(from));
    &s[a..b]
}

/// Characters from `from` to the end.
pub fn slice_from(s: &str, from: usize) -> &str {
    &s[byte_at(s, from)..]
}

/// The character at `col`, if any.
pub fn at(s: &str, col: usize) -> Option<char> {
    if s.is_ascii() {
        return s.as_bytes().get(col).map(|&b| b as char);
    }
    s.chars().nth(col)
}

/// Character column of byte index `byte` (for converting regex matches).
pub fn col_of_byte(s: &str, byte: usize) -> usize {
    if s.is_ascii() {
        return byte.min(s.len());
    }
    s[..byte.min(s.len())].chars().count()
}

/// Letters, digits and underscore: what identifiers are made of.
pub fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The run of spaces and tabs at the start of a line.
pub fn leading_whitespace(s: &str) -> &str {
    let end = s.find(|c: char| c != ' ' && c != '\t').unwrap_or(s.len());
    &s[..end]
}

/// UTF-16 length of the first `col` characters (language servers count UTF-16 units).
pub fn utf16_col(s: &str, col: usize) -> usize {
    s.chars().take(col).map(char::len_utf16).sum()
}

/// Character column of a UTF-16 offset.
pub fn col_from_utf16(s: &str, units: usize) -> usize {
    let mut seen = 0;
    for (i, ch) in s.chars().enumerate() {
        if seen >= units {
            return i;
        }
        seen += ch.len_utf16();
    }
    len(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_characters_and_bytes() {
        assert_eq!(len("héllo"), 5);
        assert_eq!(byte_at("héllo", 2), 3);
        assert_eq!(slice("héllo", 1, 3), "él");
        assert_eq!(slice_from("héllo", 4), "o");
        assert_eq!(at("héllo", 1), Some('é'));
        assert_eq!(col_of_byte("héllo", 3), 2);
        assert_eq!(byte_at("abc", 10), 3);
    }

    #[test]
    fn converts_utf16_columns() {
        // "😀" is 2 UTF-16 units but 1 character.
        assert_eq!(utf16_col("a😀b", 2), 3);
        assert_eq!(col_from_utf16("a😀b", 3), 2);
    }
}
