//! Program output: keeping it cheap (batches, and only the newest part when a
//! program prints faster than it can be shown), clickable traceback
//! locations, and stripping colour codes. Replaces `src/shared/outputBuffer.ts`
//! and `src/shared/traceback.ts`.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputPiece {
    pub stream: Stream,
    pub text: String,
}

/// Number of line breaks in `text`.
pub fn count_lines(text: &str) -> usize {
    text.bytes().filter(|b| *b == b'\n').count()
}

/// The last `max` lines of `text`, and how many lines were cut off.
pub fn last_lines(text: &str, max: usize) -> (&str, usize) {
    let total = count_lines(text);
    if total <= max {
        return (text, 0);
    }
    // `i` is the line break the kept text starts after (walking back over `max` of them).
    let mut i = text.len();
    let mut seen: isize = if text.ends_with('\n') { -1 } else { 0 };
    while i > 0 && seen < max as isize {
        match text[..i].rfind('\n') {
            Some(j) => i = j,
            None => return (text, 0),
        }
        seen += 1;
    }
    let start = (i + 1).min(text.len());
    (&text[start..], count_lines(&text[..start]))
}

/// Collects pieces of output, merging neighbours from the same stream, and
/// keeps at most `max_chars` (dropping the oldest whole lines first).
#[derive(Debug)]
pub struct OutputQueue {
    pieces: Vec<OutputPiece>,
    chars: usize,
    dropped: usize,
    max_chars: usize,
}

impl OutputQueue {
    pub fn new(max_chars: usize) -> Self {
        Self { pieces: Vec::new(), chars: 0, dropped: 0, max_chars }
    }

    pub fn size(&self) -> usize {
        self.chars
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty() && self.dropped == 0
    }

    pub fn push(&mut self, stream: Stream, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.pieces.last_mut() {
            Some(last) if last.stream == stream => last.text.push_str(text),
            _ => self.pieces.push(OutputPiece { stream, text: text.to_owned() }),
        }
        self.chars += text.len();
        if self.chars > self.max_chars {
            self.trim();
        }
    }

    /// Drop the oldest output (whole lines) until it fits.
    fn trim(&mut self) {
        // Aim a bit lower than the limit so it doesn't trim on every push.
        let target = self.max_chars * 3 / 4;
        while self.chars > target && !self.pieces.is_empty() {
            let excess = self.chars - target;
            let first = &mut self.pieces[0];
            if first.text.len() <= excess {
                self.dropped += count_lines(&first.text);
                self.chars -= first.text.len();
                self.pieces.remove(0);
                continue;
            }
            // Cut at a line break so no line is shown half.
            let from = excess.saturating_sub(1);
            let cut = first.text.get(from..).and_then(|s| s.find('\n')).map_or(first.text.len(), |i| from + i + 1);
            self.dropped += count_lines(&first.text[..cut]);
            self.chars -= cut;
            first.text.drain(..cut);
            if first.text.is_empty() {
                self.pieces.remove(0);
            }
        }
    }

    /// Everything collected (and how many lines were dropped), emptying the queue.
    pub fn take(&mut self) -> (Vec<OutputPiece>, usize) {
        let out = (std::mem::take(&mut self.pieces), self.dropped);
        self.chars = 0;
        self.dropped = 0;
        out
    }

    pub fn clear(&mut self) {
        self.take();
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn skipped_note(lines: usize) -> String {
    format!(
        "[… {} line{} of output skipped: the program printed faster than it can be shown …]\n",
        thousands(lines),
        if lines == 1 { "" } else { "s" }
    )
}

/// A `File "x.py", line 12` location in a traceback (byte offsets in the text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileLink {
    pub start: usize,
    pub end: usize,
    pub path: String,
    pub line: usize,
}

static TRACEBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"File "([^"<>]+)", line (\d+)"#).expect("regex"));
static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("regex"));

/// Python traceback lines look like:   File "C:\proj\app.py", line 12, in <module>
pub fn find_file_links(text: &str) -> Vec<FileLink> {
    TRACEBACK
        .captures_iter(text)
        .filter_map(|c| {
            let m = c.get(0)?;
            Some(FileLink { start: m.start(), end: m.end(), path: c[1].to_owned(), line: c[2].parse().ok()? })
        })
        .collect()
}

/// Remove terminal colour/style codes (the output panel shows plain text).
pub fn strip_ansi(text: &str) -> String {
    ANSI.replace_all(text, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_neighbouring_pieces_of_the_same_stream_in_order() {
        let mut q = OutputQueue::new(1000);
        q.push(Stream::Stdout, "a\n");
        q.push(Stream::Stdout, "b\n");
        q.push(Stream::Stderr, "err\n");
        q.push(Stream::Stdout, "c\n");
        let (pieces, dropped) = q.take();
        assert_eq!(
            pieces,
            vec![
                OutputPiece { stream: Stream::Stdout, text: "a\nb\n".into() },
                OutputPiece { stream: Stream::Stderr, text: "err\n".into() },
                OutputPiece { stream: Stream::Stdout, text: "c\n".into() },
            ]
        );
        assert_eq!(dropped, 0);
        assert!(q.is_empty());
    }

    #[test]
    fn keeps_only_the_newest_output_counting_dropped_lines() {
        let mut q = OutputQueue::new(100);
        for i in 0..1000 {
            q.push(Stream::Stdout, &format!("line {i:04}\n"));
        }
        assert!(q.size() <= 100);
        let (pieces, dropped) = q.take();
        let text: String = pieces.iter().map(|p| p.text.as_str()).collect();
        assert!(text.ends_with("line 0999\n"));
        assert!(text.starts_with("line "));
        assert_eq!(dropped + count_lines(&text), 1000);
    }

    #[test]
    fn drops_older_pieces_of_another_stream_too() {
        let mut q = OutputQueue::new(20);
        q.push(Stream::Stderr, "old error\n");
        q.push(Stream::Stdout, &format!("{}\n{}\n", "x".repeat(9), "y".repeat(9)));
        let (pieces, dropped) = q.take();
        assert_eq!(pieces, vec![OutputPiece { stream: Stream::Stdout, text: format!("{}\n", "y".repeat(9)) }]);
        assert_eq!(dropped, 2);
    }

    #[test]
    fn clear_forgets_everything() {
        let mut q = OutputQueue::new(10);
        q.push(Stream::Stdout, &"a".repeat(50));
        q.clear();
        assert_eq!(q.take(), (vec![], 0));
    }

    #[test]
    fn skipped_note_says_how_many_lines() {
        assert!(skipped_note(12345).contains("12,345 lines"));
        assert!(skipped_note(1).contains("1 line of"));
    }

    #[test]
    fn last_lines_keeps_the_last_lines_and_says_how_many_were_cut() {
        assert_eq!(last_lines("a\nb\nc\n", 2), ("b\nc\n", 1));
        assert_eq!(last_lines("a\nb\nc\npartial", 2), ("c\npartial", 2));
        assert_eq!(last_lines("a\nb\n", 5), ("a\nb\n", 0));
        assert_eq!(count_lines("x\ny\n"), 2);
    }

    #[test]
    fn finds_python_traceback_locations() {
        let text =
            "Traceback (most recent call last):\n  File \"C:\\proj\\app.py\", line 12, in <module>\n    main()\n";
        let links = find_file_links(text);
        assert_eq!(links[0].path, "C:\\proj\\app.py");
        assert_eq!(links[0].line, 12);
        assert_eq!(&text[links[0].start..links[0].end], "File \"C:\\proj\\app.py\", line 12");
        let two = "  File \"/a/b.py\", line 1, in f\n  File \"/a/c.py\", line 200, in g\n";
        assert_eq!(
            find_file_links(two).into_iter().map(|l| (l.path, l.line)).collect::<Vec<_>>(),
            [("/a/b.py".to_owned(), 1), ("/a/c.py".to_owned(), 200)]
        );
        assert!(find_file_links("  File \"<frozen runpy>\", line 198, in _run\n").is_empty());
    }

    #[test]
    fn strips_colour_codes() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m plain \x1b[1;32mbold\x1b[m"), "red plain bold");
    }
}
