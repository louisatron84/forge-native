//! The editor view driven like a user would: keys, typed text, the
//! clipboard. Runs headlessly (GPUI's test platform).

use super::*;
use crate::theme::{ForgeTheme, Theme};
use gpui_kit::{AppContext, TestAppContext, VisualTestContext};
use std::sync::Arc;

fn setup<'a>(cx: &'a mut TestAppContext, path: &str, text: &str) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(ForgeTheme(Arc::new(Theme::molten())));
        bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|_window, cx| EditorView::new(cx));
    let (path, text) = (path.to_owned(), text.to_owned());
    view.update_in(cx, |e, window, cx| {
        let doc = cx.new(|_| Document::new(path, &text));
        e.show(doc, cx);
        window.focus(&e.focus, cx);
    });
    cx.run_until_parked();
    (view, cx)
}

fn text(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |e, cx| e.document().map(|d| d.read(cx).doc.text()).unwrap_or_default())
}

fn cursor(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Pos {
    view.read_with(cx, |e, _| e.cursor())
}

#[gpui_kit::test]
fn typing_auto_closes_brackets_and_types_over_the_closer(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "");
    cx.simulate_input("print(x");
    assert_eq!(text(&view, cx), "print(x)");
    cx.simulate_input(")");
    assert_eq!(text(&view, cx), "print(x)");
    assert_eq!(cursor(&view, cx), Pos::new(0, 8));
}

#[gpui_kit::test]
fn enter_indents_after_a_colon_and_backspace_removes_an_indent_level(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "def f():");
    cx.simulate_keystrokes("end enter");
    assert_eq!(text(&view, cx), "def f():\n    ");
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&view, cx), "def f():\n");
}

#[gpui_kit::test]
fn shift_arrows_select_and_typing_replaces_the_selection(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "hello world");
    cx.simulate_keystrokes("shift-right shift-right shift-right shift-right shift-right");
    cx.simulate_input("bye");
    assert_eq!(text(&view, cx), "bye world");
}

#[gpui_kit::test]
fn undo_and_redo(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "");
    cx.simulate_input("abc");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("d");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&view, cx), "abc\n");
    cx.simulate_keystrokes("ctrl-z ctrl-z");
    assert_eq!(text(&view, cx), "");
    cx.simulate_keystrokes("ctrl-y ctrl-shift-z");
    assert_eq!(text(&view, cx), "abc\n");
}

#[gpui_kit::test]
fn ctrl_d_adds_the_next_occurrence_and_typing_changes_both(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "foo = foo + 1");
    cx.simulate_keystrokes("ctrl-d ctrl-d");
    assert_eq!(view.read_with(cx, |e, _| e.cursor_count()), 2);
    cx.simulate_input("bar");
    assert_eq!(text(&view, cx), "bar = bar + 1");
    cx.simulate_keystrokes("escape");
    assert_eq!(view.read_with(cx, |e, _| e.cursor_count()), 1);
}

#[gpui_kit::test]
fn copy_a_whole_line_without_a_selection_and_paste_it(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "x = 1\ny = 2");
    cx.simulate_keystrokes("ctrl-c down end enter ctrl-v");
    assert_eq!(text(&view, cx), "x = 1\ny = 2\nx = 1\n");
    assert_eq!(cx.read_from_clipboard().and_then(|i| i.text()).as_deref(), Some("x = 1\n"));
}

#[gpui_kit::test]
fn cut_removes_the_line(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "a\nb\nc");
    cx.simulate_keystrokes("down ctrl-x");
    assert_eq!(text(&view, cx), "a\nc");
    assert_eq!(cursor(&view, cx), Pos::new(1, 0));
}

#[gpui_kit::test]
fn ctrl_slash_toggles_a_comment(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "x = 1");
    cx.simulate_keystrokes("ctrl-/");
    assert_eq!(text(&view, cx), "# x = 1");
    cx.simulate_keystrokes("ctrl-/");
    assert_eq!(text(&view, cx), "x = 1");
}

#[gpui_kit::test]
fn tab_indents_selected_lines_and_shift_tab_dedents(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "a\nb");
    cx.simulate_keystrokes("ctrl-a tab");
    assert_eq!(text(&view, cx), "    a\n    b");
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(text(&view, cx), "a\nb");
}

#[gpui_kit::test]
fn folding_hides_a_block_and_moving_down_steps_over_it(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "def f():\n    x = 1\n    y = 2\nprint(f)");
    cx.simulate_keystrokes("ctrl-shift-[");
    assert_eq!(view.read_with(cx, |e, cx| e.document().map(|d| d.read(cx).doc.folds.clone())), Some(vec![0]));
    cx.simulate_keystrokes("down");
    assert_eq!(cursor(&view, cx).line, 3);
    cx.simulate_keystrokes("up ctrl-shift-]");
    assert_eq!(view.read_with(cx, |e, cx| e.document().map(|d| d.read(cx).doc.folds.len())), Some(0));
}

#[gpui_kit::test]
fn two_views_of_one_document_keep_their_own_cursors(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "first\nsecond");
    let doc = view.read_with(cx, |e, _| e.document().cloned()).expect("doc");
    let other = cx.new(EditorView::new);
    other.update(cx, |e, cx| {
        e.show(doc.clone(), cx);
        e.go_to(1, 3, cx);
    });
    // Typing a new line at the top of the first view moves the other view's cursor down.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(other.read_with(cx, |e, _| e.cursor()), Pos::new(2, 3));
    assert_eq!(cursor(&view, cx), Pos::new(1, 0));
}

#[gpui_kit::test]
fn ctrl_home_and_end_and_word_movement(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "foo.bar(baz)\nnext line");
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(cursor(&view, cx), Pos::new(1, 9));
    cx.simulate_keystrokes("ctrl-home ctrl-right ctrl-right");
    assert_eq!(cursor(&view, cx), Pos::new(0, 4));
    cx.simulate_keystrokes("ctrl-shift-right");
    assert_eq!(view.read_with(cx, |e, _| e.selection()), Selection::new(Pos::new(0, 4), Pos::new(0, 7)));
}

#[gpui_kit::test]
fn no_document_shows_the_placeholder(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, "a.py", "");
    view.update(cx, |e, cx| e.clear(Some("Open a file".into()), cx));
    cx.simulate_input("x");
    assert!(view.read_with(cx, |e, _| e.document().is_none()));
}
