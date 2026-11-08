//! Code folding: which blocks can fold (by indentation, which is how Python
//! marks blocks) and the mapping between file lines and the rows on screen
//! once some blocks are folded. Replaces `src/renderer/editor/folding.ts`.

fn indent_of(text: &str) -> usize {
    text.chars().take_while(|&c| c == ' ' || c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// True if `line` starts a block: the next non-blank line is indented deeper.
pub fn is_foldable<S: AsRef<str>>(lines: &[S], line: usize) -> bool {
    let Some(text) = lines.get(line).map(AsRef::as_ref) else {
        return false;
    };
    if is_blank(text) {
        return false;
    }
    lines[line + 1..].iter().map(AsRef::as_ref).find(|t| !is_blank(t)).is_some_and(|t| indent_of(t) > indent_of(text))
}

/// The last line of the block starting at `line` (the deeper-indented lines
/// after it, not counting trailing blank lines), or `None` if it isn't a block.
pub fn fold_end<S: AsRef<str>>(lines: &[S], line: usize) -> Option<usize> {
    let text = lines.get(line)?.as_ref();
    if is_blank(text) {
        return None;
    }
    let indent = indent_of(text);
    let mut end = None;
    for (l, t) in lines.iter().enumerate().skip(line + 1) {
        let t = t.as_ref();
        if is_blank(t) {
            continue;
        }
        if indent_of(t) <= indent {
            break;
        }
        end = Some(l);
    }
    end
}

/// The block to fold for a cursor on `line`: the line's own block if it
/// starts one, else the nearest enclosing block. Returns its header line.
pub fn enclosing_block<S: AsRef<str>>(lines: &[S], line: usize) -> Option<usize> {
    if is_foldable(lines, line) {
        return Some(line);
    }
    (0..line).rev().find(|&h| fold_end(lines, h).is_some_and(|end| end >= line))
}

/// Folded blocks of a document, and the file line <-> screen row mapping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldMap {
    /// Hidden line ranges (from, to), inclusive, sorted and not overlapping.
    ranges: Vec<(usize, usize)>,
    hidden_count: usize,
}

impl FoldMap {
    pub fn new(headers: &[usize], range_end: impl Fn(usize) -> Option<usize>) -> Self {
        let mut sorted = headers.to_vec();
        sorted.sort_unstable();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for h in sorted {
            if ranges.last().is_some_and(|&(_, to)| h <= to) {
                continue; // inside a block that's already folded away
            }
            if let Some(end) = range_end(h) {
                if end > h {
                    ranges.push((h + 1, end));
                }
            }
        }
        let hidden_count = ranges.iter().map(|(from, to)| to - from + 1).sum();
        Self { ranges, hidden_count }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn ranges(&self) -> &[(usize, usize)] {
        &self.ranges
    }

    pub fn hidden_count(&self) -> usize {
        self.hidden_count
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The folded header lines that actually hide something.
    pub fn headers(&self) -> Vec<usize> {
        self.ranges.iter().map(|(from, _)| from - 1).collect()
    }

    /// The hidden range containing `line`, if any.
    pub fn range_of(&self, line: usize) -> Option<(usize, usize)> {
        let i = self.ranges.partition_point(|&(_, to)| to < line);
        self.ranges.get(i).copied().filter(|&(from, _)| from <= line)
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        self.range_of(line).is_some()
    }

    pub fn is_folded(&self, header: usize) -> bool {
        self.range_of(header + 1).is_some_and(|(from, _)| from == header + 1)
    }

    /// The screen row of a file line (a hidden line maps to its block's header row).
    pub fn visual_line(&self, line: usize) -> usize {
        let mut hidden = 0;
        for &(from, to) in &self.ranges {
            if from > line {
                break;
            }
            if line <= to {
                return from - 1 - hidden;
            }
            hidden += to - from + 1;
        }
        line - hidden
    }

    /// The file line shown on a screen row.
    pub fn buffer_line(&self, row: usize) -> usize {
        let mut line = row;
        for &(from, to) in &self.ranges {
            if from <= line {
                line += to - from + 1;
            } else {
                break;
            }
        }
        line
    }

    pub fn visible_count(&self, line_count: usize) -> usize {
        line_count - self.hidden_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: [&str; 10] = [
        "class A:",         // 0
        "    def f(self):", // 1
        "        x = 1",    // 2
        "",                 // 3
        "        return x", // 4
        "",                 // 5
        "    def g(self):", // 6
        "        pass",     // 7
        "",                 // 8
        "print(A)",         // 9
    ];

    #[test]
    fn a_block_is_the_deeper_indented_lines_without_trailing_blank_lines() {
        assert_eq!(fold_end(&SRC, 0), Some(7));
        assert_eq!(fold_end(&SRC, 1), Some(4));
        assert_eq!(fold_end(&SRC, 6), Some(7));
    }

    #[test]
    fn lines_that_start_nothing_do_not_fold() {
        assert_eq!(fold_end(&SRC, 2), None);
        assert_eq!(fold_end(&SRC, 3), None);
        assert_eq!(fold_end(&SRC, 9), None);
        assert!(is_foldable(&SRC, 1));
        assert!(!is_foldable(&SRC, 4));
    }

    #[test]
    fn the_enclosing_block_of_a_body_line_is_its_header() {
        assert_eq!(enclosing_block(&SRC, 2), Some(1));
        assert_eq!(enclosing_block(&SRC, 1), Some(1));
        assert_eq!(enclosing_block(&SRC, 7), Some(6));
        assert_eq!(enclosing_block(&SRC, 9), None);
    }

    fn end(h: usize) -> Option<usize> {
        fold_end(&SRC, h)
    }

    #[test]
    fn maps_lines_to_screen_rows_around_a_fold() {
        let m = FoldMap::new(&[1], end); // hides 2..4
        assert_eq!(m.ranges(), [(2, 4)]);
        assert_eq!(m.visible_count(SRC.len()), 7);
        assert_eq!(m.visual_line(1), 1);
        assert_eq!(m.visual_line(3), 1);
        assert_eq!(m.visual_line(5), 2);
        assert_eq!(m.buffer_line(2), 5);
        assert_eq!(m.buffer_line(1), 1);
        assert!(m.is_hidden(4));
        assert!(m.is_folded(1));
    }

    #[test]
    fn a_fold_inside_a_folded_block_is_ignored() {
        let m = FoldMap::new(&[0, 1, 6], end);
        assert_eq!(m.ranges(), [(1, 7)]);
        assert_eq!(m.buffer_line(1), 8);
        assert_eq!(m.headers(), [0]);
    }

    #[test]
    fn several_folds() {
        let m = FoldMap::new(&[6, 1], end);
        assert_eq!(m.ranges(), [(2, 4), (7, 7)]);
        for row in 0..m.visible_count(SRC.len()) {
            assert_eq!(m.visual_line(m.buffer_line(row)), row);
        }
        assert_eq!(m.buffer_line(4), 8);
    }

    #[test]
    fn the_empty_map_changes_nothing() {
        assert_eq!(FoldMap::empty().visual_line(5), 5);
        assert_eq!(FoldMap::empty().buffer_line(5), 5);
    }
}
