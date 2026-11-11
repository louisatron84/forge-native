//! Safe marked-block edits, replacing `src/shared/boxSync.ts`.
//! Offsets are UTF-8 byte offsets into the supplied Rust string. Text outside
//! the unique pair of marker lines is always preserved byte-for-byte.
use crate::{box_code::block_hash, boxes::ApiBox};
#[derive(Clone, Debug, PartialEq)]
pub struct FoundBlock {
    pub start: usize,
    pub end: usize,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Found(FoundBlock),
    Missing,
    Broken,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlockState {
    Ok,
    Edited,
    Missing,
    Broken,
}
pub fn find_block(text: &str, id: &str) -> Block {
    let start_marker = format!("# forge:box {id}");
    let end_marker = format!("# forge:end {id}");
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let clean = line.trim_end_matches(['\r', '\n']);
        if clean
            .strip_prefix(&start_marker)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
        {
            starts.push(offset);
        }
        if clean.strip_prefix(&end_marker).is_some_and(|tail| tail.trim().is_empty()) {
            ends.push(offset + line.len());
        }
        offset += line.len();
    }
    if starts.is_empty() && ends.is_empty() {
        Block::Missing
    } else if starts.len() != 1 || ends.len() != 1 || ends[0] <= starts[0] {
        Block::Broken
    } else {
        Block::Found(FoundBlock { start: starts[0], end: ends[0], text: text[starts[0]..ends[0]].into() })
    }
}
pub fn block_state(text: &str, api: &ApiBox) -> BlockState {
    match find_block(text, &api.id) {
        Block::Missing => BlockState::Missing,
        Block::Broken => BlockState::Broken,
        Block::Found(found) => {
            if api.hash.as_deref() == Some(block_hash(&found.text).as_str()) {
                BlockState::Ok
            } else {
                BlockState::Edited
            }
        }
    }
}
pub fn with_eol(block: &str, sample: &str) -> String {
    if sample.contains("\r\n") {
        block.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        block.into()
    }
}
pub fn replace_block(text: &str, id: &str, block: &str) -> Result<String, String> {
    match find_block(text, id) {
        Block::Found(found) => Ok(format!("{}{}{}", &text[..found.start], with_eol(block, text), &text[found.end..])),
        state => {
            Err(format!("The code block for {id} is {}.", if state == Block::Missing { "missing" } else { "broken" }))
        }
    }
}
pub fn insert_block(text: &str, block: &str) -> String {
    if text.is_empty() {
        return block.into();
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    format!("{text}{}{eol}{}", if text.ends_with('\n') { "" } else { eol }, with_eol(block, text))
}
pub fn remove_block(text: &str, id: &str) -> String {
    let Block::Found(found) = find_block(text, id) else {
        return text.into();
    };
    let mut start = found.start;
    if found.end == text.len() {
        let head = &text[..start];
        if head.ends_with("\r\n\r\n") {
            start -= 2;
        } else if head.ends_with("\n\n") {
            start -= 1;
        }
    }
    format!("{}{}", &text[..start], &text[found.end..])
}
#[cfg(test)]
mod tests {
    use super::*;
    fn block(id: &str, body: &str) -> String {
        format!("# forge:box {id} (generated, edit in Forge or here)\n{body}\n# forge:end {id}\n")
    }
    #[test]
    fn finds_whole_block_including_markers_and_line_break() {
        let b = block("gh-1", "x = 1");
        let before = "import os\n\n";
        let text = format!("{before}{b}\nprint('after')\n");
        assert_eq!(
            find_block(&text, "gh-1"),
            Block::Found(FoundBlock { start: before.len(), end: before.len() + b.len(), text: b })
        );
    }
    #[test]
    fn crlf_and_no_final_newline() {
        let b = block("gh-1", "x = 1");
        let text = format!("import os\n\n{b}").replace('\n', "\r\n");
        let Block::Found(f) = find_block(&text, "gh-1") else { panic!("block") };
        assert_eq!(f.text, b.replace('\n', "\r\n"));
        let tail = text.trim_end();
        let Block::Found(f) = find_block(tail, "gh-1") else { panic!("tail") };
        assert_eq!(f.end, tail.len());
    }
    #[test]
    fn missing_broken_duplicate_and_reversed_markers() {
        assert_eq!(find_block("import os\n", "gh-1"), Block::Missing);
        let b = block("gh-1", "x = 1");
        assert_eq!(find_block(&(b.clone() + &b), "gh-1"), Block::Broken);
        assert_eq!(find_block("# forge:box gh-1 (generated)\nx = 1\n", "gh-1"), Block::Broken);
        assert_eq!(find_block("# forge:end gh-1\n", "gh-1"), Block::Broken);
        assert_eq!(find_block(&b.lines().rev().collect::<Vec<_>>().join("\n"), "gh-1"), Block::Broken);
    }
    #[test]
    fn ids_match_exactly() {
        assert_eq!(find_block(&block("gh-10", "x"), "gh-1"), Block::Missing);
        assert_eq!(find_block(&block("a.b", "x"), "axb"), Block::Missing);
    }
    #[test]
    fn state_is_ok_edited_missing_or_broken() {
        let mut api = crate::boxes::new_api_box(&[], "GH");
        let b = block(&api.id, "x = 1");
        api.hash = Some(block_hash(&b));
        assert_eq!(block_state(&b, &api), BlockState::Ok);
        assert_eq!(block_state(&b.replace('\n', "\r\n"), &api), BlockState::Ok);
        assert_eq!(block_state(&b.replace("x = 1", "x = 2"), &api), BlockState::Edited);
        assert_eq!(block_state("", &api), BlockState::Missing);
        assert_eq!(block_state(&(b.clone() + &b), &api), BlockState::Broken);
    }
    #[test]
    fn replacement_preserves_every_other_byte_including_unicode_and_crlf() {
        let before = "# λ 😀\r\nimport os\r\n\r\n";
        let after = "\r\nprint('after')\r\n";
        let old = block("gh-1", "x = 1").replace('\n', "\r\n");
        let new = block("gh-1", "y = 2");
        assert_eq!(
            replace_block(&format!("{before}{old}{after}"), "gh-1", &new).unwrap(),
            format!("{before}{}{after}", new.replace('\n', "\r\n"))
        );
    }
    #[test]
    fn insertion_appends_after_a_blank_line_and_removal_undoes_it() {
        let b = block("a", "x = 1");
        for text in ["x = 0\n", "", "x = 0\r\n"] {
            assert_eq!(remove_block(&insert_block(text, &b), "a"), text);
        }
        assert_eq!(insert_block("x = 0", &b), format!("x = 0\n\n{b}"));
        assert_eq!(with_eol("a\nb\n", "x\r\n"), "a\r\nb\r\n");
    }
}
