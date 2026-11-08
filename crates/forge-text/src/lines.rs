//! Keeping line-based markers (breakpoints, folded blocks) on their code as
//! text is edited. Replaces `src/renderer/editor/lines.ts`.

use crate::text_buffer::{Change, ChangeKind};

fn newlines(text: &str) -> usize {
    text.matches('\n').count()
}

/// Where line markers (breakpoints, folds) end up after one edit. The result
/// is sorted, without duplicates.
pub fn shift_lines(lines: &[usize], change: &Change) -> Vec<usize> {
    let n = newlines(&change.text);
    if n == 0 {
        return lines.to_vec();
    }
    let l = change.at.line;
    let mut out: Vec<usize> = Vec::with_capacity(lines.len());
    match change.kind {
        ChangeKind::Insert => {
            // Enter at the very start of a line pushes that line (and its marker) down.
            for &b in lines {
                out.push(if b > l || (b == l && change.at.col == 0) { b + n } else { b });
            }
        }
        ChangeKind::Delete if change.at.col == 0 && change.text.ends_with('\n') => {
            // Whole lines L .. L+n-1 removed (e.g. cut line): their markers go too.
            for &b in lines {
                if b < l {
                    out.push(b);
                } else if b >= l + n {
                    out.push(b - n);
                }
            }
        }
        ChangeKind::Delete => {
            // Lines L+1 .. L+n joined onto line L.
            for &b in lines {
                out.push(if b <= l {
                    b
                } else if b <= l + n {
                    l
                } else {
                    b - n
                });
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_buffer::pos;

    #[test]
    fn insert_pushes_markers_below_down() {
        let c = Change::insert(pos(1, 3), "\n\n");
        assert_eq!(shift_lines(&[0, 1, 2, 5], &c), [0, 1, 4, 7]);
    }

    #[test]
    fn enter_at_column_zero_moves_that_line_down() {
        let c = Change::insert(pos(2, 0), "\n");
        assert_eq!(shift_lines(&[2], &c), [3]);
    }

    #[test]
    fn deleting_whole_lines_removes_their_markers() {
        let c = Change::delete(pos(1, 0), "a\nb\n");
        assert_eq!(shift_lines(&[0, 1, 2, 3, 6], &c), [0, 1, 4]);
    }

    #[test]
    fn joining_lines_moves_their_markers_onto_the_joined_line() {
        let c = Change::delete(pos(1, 2), "x\ny");
        assert_eq!(shift_lines(&[1, 2, 4], &c), [1, 3]);
    }

    #[test]
    fn edits_without_line_breaks_change_nothing() {
        let c = Change::insert(pos(1, 2), "abc");
        assert_eq!(shift_lines(&[3, 1], &c), [3, 1]);
    }
}
