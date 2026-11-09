//! Pure helpers for drawing a line: tabs expanded to the next tab stop, and
//! the mapping between character columns and the drawn text.

/// A line as drawn: tabs expanded to spaces. `byte_of[col]` is the byte
/// offset in `text` where character `col` starts (one extra entry for the end).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayLine {
    pub text: String,
    pub byte_of: Vec<usize>,
}

impl DisplayLine {
    pub fn new(line: &str, tab_size: usize) -> Self {
        let mut text = String::with_capacity(line.len());
        let mut byte_of = Vec::with_capacity(line.len() + 1);
        let mut visual = 0;
        for ch in line.chars() {
            byte_of.push(text.len());
            if ch == '\t' {
                let width = tab_size - visual % tab_size;
                text.extend(std::iter::repeat_n(' ', width));
                visual += width;
            } else {
                text.push(ch);
                visual += 1;
            }
        }
        byte_of.push(text.len());
        Self { text, byte_of }
    }

    /// Byte offset in the drawn text of character column `col` (clamped).
    pub fn byte(&self, col: usize) -> usize {
        self.byte_of[col.min(self.byte_of.len() - 1)]
    }

    /// The character column at (or just before) a byte offset in the drawn text.
    pub fn col_at_byte(&self, byte: usize) -> usize {
        match self.byte_of.binary_search(&byte) {
            Ok(col) => col,
            // Inside an expanded tab: round to the nearer side.
            Err(i) => {
                let (before, after) = (self.byte_of[i - 1], self.byte_of.get(i).copied().unwrap_or(byte));
                if byte - before <= after - byte {
                    i - 1
                } else {
                    i
                }
            }
        }
    }

    /// Number of characters in the original line.
    pub fn len(&self) -> usize {
        self.byte_of.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// How many lines a page is: the visible lines less one, at least one.
pub fn page_lines(viewport_height: f32, line_height: f32) -> usize {
    if line_height <= 0.0 {
        return 1;
    }
    ((viewport_height / line_height).floor() as usize).saturating_sub(1).max(1)
}

/// The scroll offset that keeps `row` in view (nearest edge), or centred.
pub fn scroll_to_show(scroll_top: f32, row: usize, line_height: f32, viewport: f32, center: bool) -> f32 {
    let top = row as f32 * line_height;
    if center {
        return (top - (viewport - line_height) / 2.0).max(0.0);
    }
    if top < scroll_top {
        top
    } else if top + line_height > scroll_top + viewport {
        (top + line_height - viewport).max(0.0)
    } else {
        scroll_top
    }
}

/// Digits needed for the largest line number (at least 2, like the old gutter).
pub fn gutter_digits(line_count: usize) -> usize {
    line_count.max(1).to_string().len().max(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tabs_to_the_next_tab_stop() {
        let d = DisplayLine::new("a\tb", 4);
        assert_eq!(d.text, "a   b");
        assert_eq!(d.byte_of, [0, 1, 4, 5]);
        assert_eq!(d.byte(2), 4);
        assert_eq!(d.col_at_byte(4), 2);
        assert_eq!(d.col_at_byte(2), 1); // inside the tab, nearer its start
        assert_eq!(d.col_at_byte(3), 2); // nearer its end
    }

    #[test]
    fn maps_non_ascii_characters() {
        let d = DisplayLine::new("héllo", 4);
        assert_eq!(d.byte(2), 3);
        assert_eq!(d.col_at_byte(3), 2);
        assert_eq!(d.len(), 5);
    }

    #[test]
    fn pages_and_scrolling() {
        assert_eq!(page_lines(200.0, 20.0), 9);
        assert_eq!(page_lines(10.0, 20.0), 1);
        assert_eq!(scroll_to_show(0.0, 30, 20.0, 200.0, false), 420.0);
        assert_eq!(scroll_to_show(400.0, 5, 20.0, 200.0, false), 100.0);
        assert_eq!(scroll_to_show(100.0, 7, 20.0, 200.0, false), 100.0);
        assert_eq!(scroll_to_show(0.0, 50, 20.0, 200.0, true), 910.0);
    }

    #[test]
    fn gutter_fits_the_biggest_line_number() {
        assert_eq!(gutter_digits(9), 2);
        assert_eq!(gutter_digits(12_345), 5);
    }
}
