//! The code editor: Forge's own, drawn by hand. Replaces
//! `src/renderer/editor/editor.ts` (the editing logic itself is in
//! `forge-text`; this is the view: keys, mouse, IME, scrolling, drawing).
//!
//! An `EditorView` shows one `Document` at a time and keeps a cursor/scroll
//! state per document, so the same file open in both halves of a split view
//! has its own cursor on each side.

mod element;
pub mod layout;

use crate::document::{Document, DocumentEvent};
use element::{EditorElement, LayoutCache};
use forge_text::folding::{enclosing_block, fold_end, FoldMap};
use forge_text::multi_cursor::{self as mc, MultiSelection};
use forge_text::{commands as cmd, pairs, selection as sel, EditKind, Pos, Selection, TextBuffer};
use gpui_kit::{
    div, App, ClipboardItem, Context, CursorStyle, Entity, EntityId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Render, ScrollEvent, SharedString, Styled, Subscription, Window,
};
use std::collections::HashMap;

gpui_kit::actions!(
    editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        MoveWordLeft,
        MoveWordRight,
        MoveHome,
        MoveEnd,
        MoveDocStart,
        MoveDocEnd,
        MovePageUp,
        MovePageDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectHome,
        SelectEnd,
        SelectDocStart,
        SelectDocEnd,
        SelectPageUp,
        SelectPageDown,
        Backspace,
        DeleteWordLeft,
        Delete,
        DeleteWordRight,
        Newline,
        Tab,
        Untab,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        ToggleComment,
        Fold,
        Unfold,
        FoldAll,
        UnfoldAll,
        GoToBracket,
        AddNextOccurrence,
        SelectAllOccurrences,
        AddCursorAbove,
        AddCursorBelow,
        Cancel,
    ]
);

/// The key context editor bindings live in.
pub const CONTEXT: &str = "ForgeEditor";

/// The editor's own keys (the old editor.ts handled these directly; app
/// commands such as Save go through the keymap).
pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, c),
        KeyBinding::new("right", MoveRight, c),
        KeyBinding::new("up", MoveUp, c),
        KeyBinding::new("down", MoveDown, c),
        KeyBinding::new("ctrl-left", MoveWordLeft, c),
        KeyBinding::new("ctrl-right", MoveWordRight, c),
        KeyBinding::new("home", MoveHome, c),
        KeyBinding::new("end", MoveEnd, c),
        KeyBinding::new("ctrl-home", MoveDocStart, c),
        KeyBinding::new("ctrl-end", MoveDocEnd, c),
        KeyBinding::new("pageup", MovePageUp, c),
        KeyBinding::new("pagedown", MovePageDown, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, c),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("ctrl-shift-home", SelectDocStart, c),
        KeyBinding::new("ctrl-shift-end", SelectDocEnd, c),
        KeyBinding::new("shift-pageup", SelectPageUp, c),
        KeyBinding::new("shift-pagedown", SelectPageDown, c),
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("ctrl-delete", DeleteWordRight, c),
        KeyBinding::new("enter", Newline, c),
        KeyBinding::new("shift-enter", Newline, c),
        KeyBinding::new("tab", Tab, c),
        KeyBinding::new("shift-tab", Untab, c),
        KeyBinding::new("ctrl-a", SelectAll, c),
        KeyBinding::new("ctrl-c", Copy, c),
        KeyBinding::new("ctrl-insert", Copy, c),
        KeyBinding::new("ctrl-x", Cut, c),
        KeyBinding::new("shift-delete", Cut, c),
        KeyBinding::new("ctrl-v", Paste, c),
        KeyBinding::new("shift-insert", Paste, c),
        KeyBinding::new("ctrl-z", Undo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("ctrl-shift-z", Redo, c),
        KeyBinding::new("ctrl-/", ToggleComment, c),
        KeyBinding::new("ctrl-shift-[", Fold, c),
        KeyBinding::new("ctrl-shift-]", Unfold, c),
        KeyBinding::new("ctrl-shift-\\", GoToBracket, c),
        KeyBinding::new("ctrl-d", AddNextOccurrence, c),
        KeyBinding::new("ctrl-shift-l", SelectAllOccurrences, c),
        KeyBinding::new("ctrl-alt-up", AddCursorAbove, c),
        KeyBinding::new("ctrl-alt-down", AddCursorBelow, c),
        KeyBinding::new("escape", Cancel, c),
    ]);
}

/// Where this editor is in one document: cursor(s) and scroll position.
#[derive(Clone, Debug, Default)]
pub struct ViewState {
    pub cursor: Pos,
    pub anchor: Pos,
    /// Extra cursors/selections besides the main one (multi-cursor editing). Usually empty.
    pub extras: Vec<Selection>,
    pub scroll_top: f32,
    pub scroll_left: f32,
}

/// What the editor tells its owner about.
#[derive(Clone, Debug)]
pub enum EditorEvent {
    /// The shown document was edited here (or undo/redo).
    Edited,
    /// The cursor or selection moved.
    CursorMoved,
    /// The user typed text (not pasted), e.g. to open autocomplete.
    Typed(String),
    /// Ctrl+click on the text (after the cursor was placed there), e.g. go to definition.
    CtrlClick(Pos),
    /// Click in the line-number gutter (0-based line), e.g. to toggle a breakpoint.
    GutterClick(usize),
    /// The editor got keyboard focus.
    Focused,
}

impl EventEmitter<EditorEvent> for EditorView {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Autoscroll {
    Nearest,
    Center,
}

pub struct EditorView {
    focus: FocusHandle,
    document: Option<Entity<Document>>,
    views: HashMap<EntityId, ViewState>,
    subscriptions: HashMap<EntityId, Subscription>,
    placeholder: Option<SharedString>,
    /// On-screen column to aim for when moving up/down (`None` = use current).
    goal_visual: Option<usize>,
    pub tab_size: usize,
    /// Auto-close brackets and quotes while typing.
    pub auto_close: bool,
    pub font_size: f32,
    pub line_height: f32,
    pub minimap: bool,
    /// IME composition on the cursor line: character columns.
    marked: Option<(usize, usize)>,
    selecting: bool,
    dragging_minimap: bool,
    pub(crate) autoscroll: Option<Autoscroll>,
    pub(crate) layout: Option<LayoutCache>,
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EditorView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            document: None,
            views: HashMap::new(),
            subscriptions: HashMap::new(),
            placeholder: None,
            goal_visual: None,
            tab_size: 4,
            auto_close: true,
            font_size: 14.0,
            line_height: 20.0,
            minimap: true,
            marked: None,
            selecting: false,
            dragging_minimap: false,
            autoscroll: None,
            layout: None,
        }
    }

    // ---------- the shown document ----------

    pub fn document(&self) -> Option<&Entity<Document>> {
        self.document.as_ref()
    }

    /// Show a document (keeping this editor's cursor and scroll position for it).
    pub fn show(&mut self, document: Entity<Document>, cx: &mut Context<Self>) {
        let id = document.entity_id();
        self.views.entry(id).or_default();
        if let std::collections::hash_map::Entry::Vacant(slot) = self.subscriptions.entry(id) {
            let sub = cx.subscribe(&document, move |this, doc, event: &DocumentEvent, cx| {
                if let DocumentEvent::Edited { origin, changes } = event {
                    // Edits made elsewhere (the other side of a split, or code) shift this view's cursors along.
                    if *origin != Some(cx.entity_id()) {
                        if let Some(v) = this.views.get_mut(&doc.entity_id()) {
                            for change in changes {
                                v.cursor = mc::map_position(v.cursor, change, true);
                                v.anchor = mc::map_position(v.anchor, change, true);
                                for s in &mut v.extras {
                                    s.anchor = mc::map_position(s.anchor, change, true);
                                    s.head = mc::map_position(s.head, change, true);
                                }
                            }
                        }
                    }
                }
                cx.notify();
            });
            slot.insert(sub);
        }
        self.document = Some(document);
        self.placeholder = None;
        self.goal_visual = None;
        self.marked = None;
        cx.notify();
    }

    /// Show nothing (all files closed), optionally with a message.
    pub fn clear(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
        self.document = None;
        self.placeholder = message;
        cx.notify();
    }

    /// Forget this editor's state for a closed document.
    pub fn forget(&mut self, document: &Entity<Document>) {
        let id = document.entity_id();
        self.views.remove(&id);
        self.subscriptions.remove(&id);
        if self.document.as_ref().map(|d| d.entity_id()) == Some(id) {
            self.document = None;
        }
    }

    pub fn placeholder(&self) -> Option<&SharedString> {
        self.placeholder.as_ref()
    }

    fn doc_id(&self) -> Option<EntityId> {
        self.document.as_ref().map(|d| d.entity_id())
    }

    pub fn view(&self) -> ViewState {
        self.doc_id().and_then(|id| self.views.get(&id)).cloned().unwrap_or_default()
    }

    pub(crate) fn view_mut(&mut self) -> Option<&mut ViewState> {
        let id = self.doc_id()?;
        self.views.get_mut(&id)
    }

    /// The main cursor.
    pub fn cursor(&self) -> Pos {
        self.view().cursor
    }

    /// The main selection.
    pub fn selection(&self) -> Selection {
        let v = self.view();
        Selection::new(v.anchor, v.cursor)
    }

    /// How many cursors there are (1 normally).
    pub fn cursor_count(&self) -> usize {
        1 + self.view().extras.len()
    }

    pub fn selected_text(&self, cx: &App) -> String {
        let Some(d) = &self.document else { return String::new() };
        let s = self.selection();
        d.read(cx).doc.buffer.range(s.anchor, s.head)
    }

    /// Move the cursor to a position (e.g. Go to Line, a search result).
    pub fn go_to(&mut self, line: usize, col: usize, cx: &mut Context<Self>) {
        let Some(d) = &self.document else { return };
        let p = d.read(cx).doc.buffer.clamp(Pos::new(line, col));
        self.unfold_around(p.line, cx);
        if let Some(v) = self.view_mut() {
            v.cursor = p;
            v.anchor = p;
            v.extras.clear();
        }
        self.goal_visual = None;
        self.autoscroll = Some(Autoscroll::Center);
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    /// Select a range (e.g. a find match).
    pub fn select(&mut self, anchor: Pos, head: Pos, cx: &mut Context<Self>) {
        self.unfold_around(head.line, cx);
        if let Some(v) = self.view_mut() {
            v.anchor = anchor;
            v.cursor = head;
            v.extras.clear();
        }
        self.autoscroll = Some(Autoscroll::Center);
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    fn all_selections(&self) -> Vec<MultiSelection> {
        let v = self.view();
        let mut all = vec![MultiSelection::new(v.anchor, v.cursor, true)];
        all.extend(v.extras.iter().map(|s| MultiSelection::new(s.anchor, s.head, false)));
        all
    }

    /// Replace all cursors; the one marked primary becomes the main cursor.
    fn set_selections(&mut self, list: &[MultiSelection]) {
        let all = mc::normalize(list);
        let Some(primary) = all.iter().position(|s| s.primary) else { return };
        let Some(v) = self.view_mut() else { return };
        v.anchor = all[primary].anchor;
        v.cursor = all[primary].head;
        v.extras = all.iter().enumerate().filter(|(i, _)| *i != primary).map(|(_, s)| s.sel()).collect();
    }

    fn multi(&self) -> bool {
        !self.view().extras.is_empty()
    }

    fn clear_extras(&mut self) {
        if let Some(v) = self.view_mut() {
            v.extras.clear();
        }
    }

    fn moved(&mut self, cx: &mut Context<Self>) {
        self.autoscroll = Some(Autoscroll::Nearest);
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    // ---------- editing ----------

    /// Run an edit with one cursor as one undo step; `op` returns (cursor, anchor).
    fn edit_single(
        &mut self,
        kind: EditKind,
        cx: &mut Context<Self>,
        op: impl FnOnce(&mut TextBuffer, Selection) -> (Pos, Pos),
    ) {
        let Some(document) = self.document.clone() else { return };
        let s = self.selection();
        let me = cx.entity_id();
        let (cursor, anchor) = document.update(cx, |d, cx| {
            d.edit(Some(me), cx, |doc| {
                let mut anchor = s.head;
                let cursor = doc.history.transact(&mut doc.buffer, kind, s.head, |b| {
                    let (c, a) = op(b, s);
                    anchor = a;
                    c
                });
                (cursor, anchor)
            })
        });
        self.after_edit(cursor, anchor, cx);
    }

    /// Run an edit at every cursor as one undo step.
    fn edit_multi(
        &mut self,
        kind: EditKind,
        cx: &mut Context<Self>,
        mut op: impl FnMut(&mut TextBuffer, Selection, usize) -> Selection,
    ) {
        let Some(document) = self.document.clone() else { return };
        let all = self.all_selections();
        let cursor = self.cursor();
        let me = cx.entity_id();
        let result = document.update(cx, |d, cx| {
            d.edit(Some(me), cx, |doc| {
                let mut result = Vec::new();
                doc.history.transact(&mut doc.buffer, kind, cursor, |b| {
                    result = mc::edit_all(b, &all, &mut op);
                    result.iter().find(|s| s.primary).map_or(cursor, |s| s.head)
                });
                result
            })
        });
        if result.is_empty() {
            return;
        }
        self.set_selections(&result);
        self.goal_visual = None;
        self.autoscroll = Some(Autoscroll::Nearest);
        cx.emit(EditorEvent::Edited);
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    /// After any edit: collapse the selection, redraw.
    fn after_edit(&mut self, cursor: Pos, anchor: Pos, cx: &mut Context<Self>) {
        if let Some(v) = self.view_mut() {
            v.cursor = cursor;
            v.anchor = anchor;
        }
        self.goal_visual = None;
        self.autoscroll = Some(Autoscroll::Nearest);
        cx.emit(EditorEvent::Edited);
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    fn mode_info(&self, cx: &App) -> (fn(&str) -> bool, bool, Option<&'static str>) {
        match &self.document {
            Some(d) => {
                let m = d.read(cx).doc.mode();
                (m.indent_after, m.string_prefixes, m.line_comment)
            }
            None => (cmd::no_indent_after, false, None),
        }
    }

    /// Typed or pasted text (from the keyboard, IME or the clipboard).
    pub fn insert_text(&mut self, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        if self.document.is_none() || text.is_empty() {
            return;
        }
        let (_, prefixes, _) = self.mode_info(cx);
        let auto_close = self.auto_close;
        let single_char = text.chars().count() == 1;
        let ch = text.chars().next().unwrap_or(' ');
        if self.multi() {
            // Pasting as many lines as there are cursors puts one line at each cursor.
            let normalized = text.replace("\r\n", "\n");
            let lines: Vec<String> =
                normalized.strip_suffix('\n').unwrap_or(&normalized).split('\n').map(str::to_owned).collect();
            let per_cursor = kind == EditKind::Paste && lines.len() == self.cursor_count();
            self.edit_multi(kind, cx, |b, s, i| {
                if kind == EditKind::Typing && single_char && auto_close {
                    if let Some(paired) = pairs::type_char(b, s, ch, prefixes) {
                        return paired;
                    }
                }
                let at = if s.is_empty() { s.head } else { sel::delete_selection(b, s) };
                let piece = if per_cursor { lines[i].as_str() } else { text };
                Selection::cursor(b.insert(at, piece))
            });
            return; // no autocomplete with several cursors
        }
        // Typing over a selection is a normal 'typing' step, so the letters typed after it
        // merge in and one Ctrl+Z brings the original selection text back.
        self.edit_single(kind, cx, |b, s| {
            if kind == EditKind::Typing && single_char && auto_close {
                if let Some(paired) = pairs::type_char(b, s, ch, prefixes) {
                    return (paired.head, paired.anchor);
                }
            }
            let at = if s.is_empty() { s.head } else { sel::delete_selection(b, s) };
            let end = b.insert(at, text);
            (end, end)
        });
        if kind == EditKind::Typing {
            cx.emit(EditorEvent::Typed(text.to_owned()));
        }
    }

    fn backspace_impl(&mut self, word: bool, cx: &mut Context<Self>) {
        let tab = self.tab_size;
        let auto_close = self.auto_close;
        let op = move |b: &mut TextBuffer, s: Selection, _: usize| -> Selection {
            if !s.is_empty() {
                return Selection::cursor(sel::delete_selection(b, s));
            }
            if word {
                let start = sel::word_left(b, s.head);
                b.delete(start, s.head);
                return Selection::cursor(start);
            }
            let p = if auto_close { pairs::backspace_pair(b, s.head) } else { None };
            Selection::cursor(p.unwrap_or_else(|| cmd::backspace(b, s.head, tab)))
        };
        self.edit_multi(EditKind::Delete, cx, op);
    }

    fn delete_impl(&mut self, word: bool, cx: &mut Context<Self>) {
        self.edit_multi(EditKind::Delete, cx, move |b, s, _| {
            if !s.is_empty() {
                return Selection::cursor(sel::delete_selection(b, s));
            }
            if word {
                let end = sel::word_right(b, s.head);
                b.delete(s.head, end);
            } else {
                cmd::delete_forward(b, s.head);
            }
            Selection::cursor(s.head)
        });
    }

    fn newline_impl(&mut self, cx: &mut Context<Self>) {
        let tab = self.tab_size;
        let (indent_after, _, _) = self.mode_info(cx);
        self.edit_multi(EditKind::Newline, cx, move |b, s, _| {
            if s.is_empty() {
                if let Some(p) = pairs::newline_between_brackets(b, s.head, tab) {
                    return Selection::cursor(p);
                }
            }
            let at = if s.is_empty() { s.head } else { sel::delete_selection(b, s) };
            Selection::cursor(cmd::newline(b, at, tab, indent_after))
        });
    }

    fn tab_impl(&mut self, dedent: bool, cx: &mut Context<Self>) {
        let tab = self.tab_size;
        self.edit_multi(EditKind::Other, cx, move |b, s, _| {
            let (first, last) = sel::selected_lines(s);
            if dedent {
                return sel::dedent_lines(b, s, tab);
            }
            if !s.is_empty() && first != last {
                return sel::indent_lines(b, s, tab);
            }
            let at = if s.is_empty() { s.head } else { sel::delete_selection(b, s) };
            Selection::cursor(cmd::insert_tab(b, at, tab))
        });
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        self.history_step(true, cx);
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        self.history_step(false, cx);
    }

    fn history_step(&mut self, undo: bool, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else { return };
        let me = cx.entity_id();
        let at = document.update(cx, |d, cx| {
            d.edit(Some(me), cx, |doc| {
                if undo {
                    doc.history.undo(&mut doc.buffer)
                } else {
                    doc.history.redo(&mut doc.buffer)
                }
            })
        });
        if let Some(p) = at {
            self.clear_extras();
            self.after_edit(p, p, cx);
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &self.document else { return };
        let end = d.read(cx).doc.buffer.end();
        self.clear_extras();
        if let Some(v) = self.view_mut() {
            v.anchor = Pos::new(0, 0);
            v.cursor = end;
        }
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    /// Ctrl+C / Ctrl+X. With nothing selected, copies/cuts the whole line (VS Code style).
    fn copy_or_cut(&mut self, cut: bool, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else { return };
        if self.multi() {
            // One piece per cursor, joined by newlines.
            let all = mc::normalize(&self.all_selections());
            let text = {
                let b = &document.read(cx).doc.buffer;
                all.iter()
                    .map(|s| if s.is_empty() { b.line(s.head.line).to_owned() } else { b.range(s.anchor, s.head) })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            if cut {
                self.edit_multi(EditKind::Other, cx, |b, s, _| {
                    if !s.is_empty() {
                        return Selection::cursor(sel::delete_selection(b, s));
                    }
                    let line = s.head.line;
                    let last = line == b.line_count() - 1;
                    if last && line > 0 {
                        b.delete(Pos::new(line - 1, b.line_len(line - 1)), Pos::new(line, b.line_len(line)));
                    } else if last {
                        b.delete(Pos::new(line, 0), Pos::new(line, b.line_len(line)));
                    } else {
                        b.delete(Pos::new(line, 0), Pos::new(line + 1, 0));
                    }
                    Selection::cursor(b.clamp(Pos::new(line, 0)))
                });
            }
            return;
        }
        let s = self.selection();
        let p = s.head;
        let (text, start, end) = {
            let b = &document.read(cx).doc.buffer;
            if !s.is_empty() {
                let (start, end) = s.range();
                (b.range(start, end), start, end)
            } else {
                let last = p.line == b.line_count() - 1;
                let start =
                    if last && p.line > 0 { Pos::new(p.line - 1, b.line_len(p.line - 1)) } else { Pos::new(p.line, 0) };
                let end = if last { Pos::new(p.line, b.line_len(p.line)) } else { Pos::new(p.line + 1, 0) };
                (format!("{}\n", b.line(p.line)), start, end)
            }
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if !cut {
            return;
        }
        let whole_line = s.is_empty();
        self.edit_single(EditKind::Other, cx, move |b, _| {
            b.delete(start, end);
            // After cutting a whole line, the cursor goes to the start of the line that moved up.
            let c = if whole_line { b.clamp(Pos::new(p.line, 0)) } else { start };
            (c, c)
        });
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        self.insert_text(&text, EditKind::Paste, cx);
    }

    /// Ctrl+/: comment or uncomment the selected lines (at every cursor).
    pub fn toggle_comment(&mut self, cx: &mut Context<Self>) {
        let (_, _, marker) = self.mode_info(cx);
        let Some(marker) = marker else { return };
        // Several cursors on one line must toggle it once, not once per cursor.
        let mut done = std::collections::HashSet::new();
        self.edit_multi(EditKind::Other, cx, move |b, s, _| {
            let (first, last) = sel::selected_lines(s);
            if (first..=last).all(|l| done.contains(&l)) {
                return s;
            }
            done.extend(first..=last);
            pairs::toggle_comment(b, s, marker)
        });
    }

    // ---------- movement ----------

    fn page_lines(&self) -> isize {
        let height = self.layout.as_ref().map_or(600.0, |l| l.viewport_height);
        layout::page_lines(height, self.line_height) as isize
    }

    fn fold_map(&self, cx: &App) -> FoldMap {
        match &self.document {
            Some(d) => {
                let doc = &d.read(cx).doc;
                if doc.folds.is_empty() {
                    FoldMap::empty()
                } else {
                    FoldMap::new(&doc.folds, |h| fold_end(doc.buffer.lines(), h))
                }
            }
            None => FoldMap::empty(),
        }
    }

    fn vertical(&mut self, delta: isize, cx: &App) -> Pos {
        let Some(d) = &self.document else { return Pos::default() };
        let b = &d.read(cx).doc.buffer;
        let cursor = self.cursor();
        let goal =
            *self.goal_visual.get_or_insert_with(|| cmd::visual_col(b.line(cursor.line), cursor.col, self.tab_size));
        let folds = self.fold_map(cx);
        let mut delta = delta;
        if !folds.is_empty() {
            // Move by rows on screen, so folded blocks are stepped over.
            let rows = folds.visible_count(b.line_count()) as isize;
            let row = folds.visual_line(cursor.line) as isize + delta;
            if (0..rows).contains(&row) {
                delta = folds.buffer_line(row as usize) as isize - cursor.line as isize;
            }
        }
        cmd::move_vertical(b, cursor, delta, goal, self.tab_size)
    }

    fn move_cursor(&mut self, motion: Motion, extend: bool, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else { return };
        let tab = self.tab_size;
        if self.multi() {
            if matches!(motion, Motion::DocStart | Motion::DocEnd | Motion::PageUp | Motion::PageDown) {
                self.clear_extras(); // jump with one cursor
            } else {
                let moved: Vec<MultiSelection> = {
                    let b = &document.read(cx).doc.buffer;
                    self.all_selections()
                        .into_iter()
                        .map(|s| {
                            let p = s.head;
                            let has = !s.is_empty();
                            let vertical = |delta: isize| {
                                cmd::move_vertical(b, p, delta, cmd::visual_col(b.line(p.line), p.col, tab), tab)
                            };
                            let t = match motion {
                                Motion::Left if !extend && has => s.sel().start(),
                                Motion::Right if !extend && has => s.sel().end(),
                                Motion::Left => cmd::move_left(b, p),
                                Motion::Right => cmd::move_right(b, p),
                                Motion::WordLeft => sel::word_left(b, p),
                                Motion::WordRight => sel::word_right(b, p),
                                Motion::Up => vertical(-1),
                                Motion::Down => vertical(1),
                                Motion::Home => cmd::move_home(b, p),
                                Motion::End => cmd::move_end(b, p),
                                _ => p,
                            };
                            MultiSelection::new(if extend { s.anchor } else { t }, t, s.primary)
                        })
                        .collect()
                };
                self.set_selections(&moved);
                self.goal_visual = None;
                self.moved(cx);
                return;
            }
        }
        let s = self.selection();
        let p = s.head;
        let has = !s.is_empty();
        let mut vertical = false;
        let target = match motion {
            Motion::Up => {
                vertical = true;
                self.vertical(-1, cx)
            }
            Motion::Down => {
                vertical = true;
                self.vertical(1, cx)
            }
            Motion::PageUp => {
                vertical = true;
                let n = self.page_lines();
                self.vertical(-n, cx)
            }
            Motion::PageDown => {
                vertical = true;
                let n = self.page_lines();
                self.vertical(n, cx)
            }
            _ => {
                let b = &document.read(cx).doc.buffer;
                match motion {
                    Motion::Left if !extend && has => s.start(),
                    Motion::Right if !extend && has => s.end(),
                    Motion::Left => cmd::move_left(b, p),
                    Motion::Right => cmd::move_right(b, p),
                    Motion::WordLeft => sel::word_left(b, p),
                    Motion::WordRight => sel::word_right(b, p),
                    Motion::Home => cmd::move_home(b, p),
                    Motion::End => cmd::move_end(b, p),
                    Motion::DocStart => Pos::new(0, 0),
                    Motion::DocEnd => b.end(),
                    _ => p,
                }
            }
        };
        if !vertical {
            self.goal_visual = None;
        }
        if let Some(v) = self.view_mut() {
            v.cursor = target;
            if !extend {
                v.anchor = target;
            }
        }
        self.moved(cx);
    }

    // ---------- folding ----------

    fn set_folds(&mut self, headers: Vec<usize>, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else { return };
        document.update(cx, |d, cx| {
            let mut h = headers;
            h.sort_unstable();
            h.dedup();
            d.doc.folds = h;
            cx.emit(DocumentEvent::Changed);
            cx.notify();
        });
        // A cursor inside a folded block moves to its header.
        let folds = self.fold_map(cx);
        if let Some(v) = self.view_mut() {
            if let Some((from, _)) = folds.range_of(v.cursor.line) {
                v.cursor = Pos::new(from - 1, 0);
                v.anchor = v.cursor;
            }
        }
        self.autoscroll = Some(Autoscroll::Nearest);
        cx.notify();
    }

    /// Fold the block at (or around) `line`.
    pub fn fold(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(d) = &self.document else { return };
        let (header, mut folds) = {
            let doc = &d.read(cx).doc;
            (enclosing_block(doc.buffer.lines(), line), doc.folds.clone())
        };
        if let Some(h) = header {
            if !folds.contains(&h) {
                folds.push(h);
                self.set_folds(folds, cx);
            }
        }
    }

    /// Unfold the block at `line` (or the folded block hiding it).
    pub fn unfold(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(d) = &self.document else { return };
        let folds = d.read(cx).doc.folds.clone();
        let map = self.fold_map(cx);
        let target = if map.is_folded(line) {
            Some(line)
        } else if let Some((from, _)) = map.range_of(line) {
            Some(from - 1)
        } else {
            let lines = d.read(cx).doc.buffer.lines().to_vec();
            enclosing_block(&lines, line).filter(|h| folds.contains(h))
        };
        if let Some(t) = target {
            self.set_folds(folds.into_iter().filter(|&h| h != t).collect(), cx);
        }
    }

    pub fn toggle_fold(&mut self, line: usize, cx: &mut Context<Self>) {
        if self.fold_map(cx).is_folded(line) {
            self.unfold(line, cx);
        } else {
            self.fold(line, cx);
        }
    }

    pub fn fold_all(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &self.document else { return };
        let headers: Vec<usize> = {
            let lines = d.read(cx).doc.buffer.lines();
            (0..lines.len()).filter(|&l| forge_text::folding::is_foldable(lines, l)).collect()
        };
        self.set_folds(headers, cx);
    }

    pub fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.set_folds(Vec::new(), cx);
    }

    /// Make sure `line` isn't hidden in a folded block.
    fn unfold_around(&mut self, line: usize, cx: &mut Context<Self>) {
        let map = self.fold_map(cx);
        if map.is_hidden(line) {
            let Some(d) = &self.document else { return };
            let folds: Vec<usize> = d
                .read(cx)
                .doc
                .folds
                .iter()
                .copied()
                .filter(|&h| map.range_of(h + 1).is_none_or(|(from, to)| line < from || line > to))
                .collect();
            self.set_folds(folds, cx);
        }
    }

    // ---------- multi-cursor ----------

    fn add_next_occurrence(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.document.clone() else { return };
        let s = self.selection();
        if s.is_empty() {
            // Select the word first.
            let line = d.read(cx).doc.buffer.line(s.head.line).to_owned();
            let (start, end) = sel::word_at(&line, s.head.col);
            if start == end {
                return;
            }
            if let Some(v) = self.view_mut() {
                v.anchor = Pos::new(s.head.line, start);
                v.cursor = Pos::new(s.head.line, end);
            }
        } else {
            let text = self.selected_text(cx);
            let all = self.all_selections();
            let existing: Vec<Selection> = all.iter().map(|m| m.sel()).collect();
            let found = mc::next_occurrence(&d.read(cx).doc.buffer, &text, s.end(), &existing);
            if let Some(f) = found {
                let mut list: Vec<MultiSelection> =
                    all.iter().map(|m| MultiSelection::new(m.anchor, m.head, false)).collect();
                list.push(MultiSelection::new(f.anchor, f.head, true));
                self.set_selections(&list);
            }
        }
        self.autoscroll = Some(Autoscroll::Center);
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    fn select_all_occurrences(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.document.clone() else { return };
        let mut text = self.selected_text(cx);
        let cursor = self.cursor();
        if text.is_empty() {
            let line = d.read(cx).doc.buffer.line(cursor.line).to_owned();
            let (start, end) = sel::word_at(&line, cursor.col);
            text = forge_text::chars::slice(&line, start, end).to_owned();
        }
        let found = mc::all_occurrences(&d.read(cx).doc.buffer, &text);
        if found.is_empty() {
            return;
        }
        let here = found.iter().position(|s| s.anchor <= cursor && cursor <= s.head).unwrap_or(0);
        let list: Vec<MultiSelection> =
            found.iter().enumerate().map(|(i, s)| MultiSelection::new(s.anchor, s.head, i == here)).collect();
        self.set_selections(&list);
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
    }

    fn add_cursor_vertical(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(d) = self.document.clone() else { return };
        let all = mc::normalize(&self.all_selections());
        let edge = if delta < 0 { all[0] } else { all[all.len() - 1] };
        let target = {
            let b = &d.read(cx).doc.buffer;
            let line = edge.head.line as isize + delta;
            if line < 0 || line >= b.line_count() as isize {
                return;
            }
            let goal = cmd::visual_col(b.line(edge.head.line), edge.head.col, self.tab_size);
            cmd::move_vertical(b, edge.head, delta, goal, self.tab_size)
        };
        let mut list: Vec<MultiSelection> = all.iter().map(|m| MultiSelection::new(m.anchor, m.head, false)).collect();
        list.push(MultiSelection::new(target, target, true));
        self.set_selections(&list);
        self.moved(cx);
    }

    fn go_to_bracket(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.bracket_match(cx) {
            self.clear_extras();
            if let Some(v) = self.view_mut() {
                v.cursor = m.matching;
                v.anchor = m.matching;
            }
            self.moved(cx);
        }
    }

    /// The bracket next to the cursor and its partner (ignoring brackets in strings and comments).
    pub(crate) fn bracket_match(&self, cx: &mut App) -> Option<pairs::BracketMatch> {
        let d = self.document.clone()?;
        let cursor = self.cursor();
        d.update(cx, |d, _| {
            let doc = &mut d.doc;
            let mode = doc.mode();
            let buffer = &doc.buffer;
            // Tokens are fetched lazily, only for the lines the search actually reaches.
            let highlighter = std::cell::RefCell::new(&mut doc.highlighter);
            let cache: std::cell::RefCell<HashMap<usize, Vec<forge_text::Token>>> = Default::default();
            let is_code = |l: usize, c: usize| {
                let mut cache = cache.borrow_mut();
                let tokens = cache.entry(l).or_insert_with(|| highlighter.borrow_mut().tokens_for(buffer, mode, l));
                !tokens.iter().any(|t| {
                    t.start <= c
                        && c < t.end
                        && matches!(t.kind, forge_text::TokenType::String | forge_text::TokenType::Comment)
                })
            };
            pairs::find_matching_bracket(buffer, cursor, &is_code, 1000)
        })
    }

    // ---------- mouse ----------

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.emit(EditorEvent::Focused);
        let Some(layout) = self.layout.clone() else { return };
        let Some(document) = self.document.clone() else { return };
        // The minimap: click or drag to scroll.
        if self.minimap && layout.in_minimap(event.position) {
            self.dragging_minimap = true;
            self.scroll_minimap_to(event.position.y, cx);
            return;
        }
        let hit = layout.hit_test(event.position, &document.read(cx).doc.buffer, self.tab_size);
        if hit.in_gutter {
            if hit.on_fold_marker {
                self.toggle_fold(hit.pos.line, cx);
            } else {
                cx.emit(EditorEvent::GutterClick(hit.pos.line));
            }
            return;
        }
        if hit.on_fold_placeholder {
            self.unfold(hit.pos.line, cx);
            return;
        }
        let pos = hit.pos;
        let m = event.modifiers;
        // Alt+click: add another cursor (it becomes the main one).
        if m.alt && !m.shift && event.click_count == 1 {
            let mut list: Vec<MultiSelection> =
                self.all_selections().iter().map(|s| MultiSelection::new(s.anchor, s.head, false)).collect();
            list.push(MultiSelection::new(pos, pos, true));
            self.set_selections(&list);
            self.goal_visual = None;
            cx.emit(EditorEvent::CursorMoved);
            cx.notify();
            return;
        }
        self.clear_extras();
        let line_text = document.read(cx).doc.buffer.line(pos.line).to_owned();
        let line_count = document.read(cx).doc.buffer.line_count();
        if let Some(v) = self.view_mut() {
            if event.click_count == 2 {
                let (start, end) = sel::word_at(&line_text, pos.col);
                v.anchor = Pos::new(pos.line, start);
                v.cursor = Pos::new(pos.line, end);
            } else if event.click_count >= 3 {
                v.anchor = Pos::new(pos.line, 0);
                v.cursor = if pos.line + 1 < line_count {
                    Pos::new(pos.line + 1, 0)
                } else {
                    Pos::new(pos.line, forge_text::chars::len(&line_text))
                };
            } else {
                v.cursor = pos;
                if !m.shift {
                    v.anchor = pos;
                }
            }
        }
        self.goal_visual = None;
        self.selecting = event.click_count == 1;
        cx.emit(EditorEvent::CursorMoved);
        cx.notify();
        if (m.control || m.platform) && event.click_count == 1 && !m.shift {
            self.selecting = false;
            cx.emit(EditorEvent::CtrlClick(pos));
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            self.dragging_minimap = false;
            return;
        }
        if self.dragging_minimap {
            self.scroll_minimap_to(event.position.y, cx);
            return;
        }
        if !self.selecting {
            return;
        }
        let (Some(layout), Some(document)) = (self.layout.clone(), self.document.clone()) else { return };
        let hit = layout.hit_test(event.position, &document.read(cx).doc.buffer, self.tab_size);
        if hit.pos != self.cursor() {
            if let Some(v) = self.view_mut() {
                v.cursor = hit.pos;
            }
            self.autoscroll = Some(Autoscroll::Nearest);
            cx.emit(EditorEvent::CursorMoved);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.selecting = false;
        self.dragging_minimap = false;
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let line_height = gpui_kit::px(self.line_height);
        let delta = event.delta.pixel_delta(line_height);
        let (max_top, max_left) = self.layout.as_ref().map_or((0.0, 0.0), |l| (l.max_scroll_top, l.max_scroll_left));
        if let Some(v) = self.view_mut() {
            v.scroll_top = (v.scroll_top - f32::from(delta.y)).clamp(0.0, max_top);
            v.scroll_left = (v.scroll_left - f32::from(delta.x)).clamp(0.0, max_left);
        }
        cx.notify();
    }

    fn scroll_minimap_to(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else { return };
        let row = layout.minimap_row_at(y);
        let top = (row as f32 * self.line_height - layout.viewport_height / 2.0).clamp(0.0, layout.max_scroll_top);
        if let Some(v) = self.view_mut() {
            v.scroll_top = top;
        }
        cx.notify();
    }
}

// ---------- IME and text input ----------
//
// The input handler's coordinates are UTF-16 offsets within the cursor's
// line: compositions and character pickers work inside one line.

impl gpui_kit::EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        actual: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let d = self.document.as_ref()?;
        let line = d.read(cx).doc.buffer.line(self.cursor().line).to_owned();
        let units: Vec<u16> = line.encode_utf16().collect();
        let start = range.start.min(units.len());
        let end = range.end.min(units.len());
        actual.replace(start..end);
        Some(String::from_utf16_lossy(&units[start..end]))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::UTF16Selection> {
        let d = self.document.as_ref()?;
        let s = self.selection();
        let line = d.read(cx).doc.buffer.line(s.head.line).to_owned();
        let head = forge_text::chars::utf16_col(&line, s.head.col);
        let anchor =
            if s.anchor.line == s.head.line { forge_text::chars::utf16_col(&line, s.anchor.col) } else { head };
        Some(gpui_kit::UTF16Selection { range: anchor.min(head)..anchor.max(head), reversed: head < anchor })
    }

    fn marked_text_range(&self, _window: &mut Window, cx: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        let (start, end) = self.marked?;
        let d = self.document.as_ref()?;
        let line = d.read(cx).doc.buffer.line(self.cursor().line).to_owned();
        Some(forge_text::chars::utf16_col(&line, start)..forge_text::chars::utf16_col(&line, end))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = range.map(|r| self.cols_from_utf16(r, cx)).or(self.marked);
        self.marked = None;
        match target {
            // Replacing a given range on the line (IME commit, character picker).
            Some((start, end)) => {
                let line = self.cursor().line;
                self.edit_single(EditKind::Typing, cx, move |b, _| {
                    b.delete(Pos::new(line, start), Pos::new(line, end));
                    let p = b.insert(Pos::new(line, start), text);
                    (p, p)
                });
            }
            // Ordinary typing.
            None => self.insert_text(text, EditKind::Typing, cx),
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        new_selected: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = self.cursor().line;
        let (start, end) = match range.map(|r| self.cols_from_utf16(r, cx)).or(self.marked) {
            Some(r) => r,
            None => {
                let s = self.selection();
                if s.anchor.line == s.head.line {
                    (s.start().col, s.end().col)
                } else {
                    (s.head.col, s.head.col)
                }
            }
        };
        let len = text.chars().count();
        self.edit_single(EditKind::Typing, cx, move |b, _| {
            b.delete(Pos::new(line, start), Pos::new(line, end));
            let p = b.insert(Pos::new(line, start), text);
            (p, p)
        });
        self.marked = (len > 0).then_some((start, start + len));
        if let Some(sel_range) = new_selected {
            // The IME's own cursor inside the composition (UTF-16, relative to the composed text).
            let units: Vec<u16> = text.encode_utf16().collect();
            let rel = String::from_utf16_lossy(&units[..sel_range.end.min(units.len())]).chars().count();
            if let Some(v) = self.view_mut() {
                v.cursor = Pos::new(line, start + rel);
                v.anchor = v.cursor;
            }
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        _element_bounds: gpui_kit::Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Bounds<Pixels>> {
        let (start, end) = self.cols_from_utf16(range, cx);
        let line = self.cursor().line;
        self.layout.as_ref()?.bounds_for_cols(line, start, end)
    }

    fn character_index_for_point(
        &mut self,
        point: gpui_kit::Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.clone()?;
        let d = self.document.clone()?;
        let hit = layout.hit_test(point, &d.read(cx).doc.buffer, self.tab_size);
        if hit.pos.line != self.cursor().line {
            return None;
        }
        let line = d.read(cx).doc.buffer.line(hit.pos.line).to_owned();
        Some(forge_text::chars::utf16_col(&line, hit.pos.col))
    }
}

impl EditorView {
    fn cols_from_utf16(&self, range: std::ops::Range<usize>, cx: &App) -> (usize, usize) {
        let Some(d) = &self.document else { return (0, 0) };
        let line = d.read(cx).doc.buffer.line(self.cursor().line).to_owned();
        (forge_text::chars::col_from_utf16(&line, range.start), forge_text::chars::col_from_utf16(&line, range.end))
    }
}

macro_rules! motion_handlers {
    ($($action:ident => $motion:ident, $extend:expr;)*) => {
        impl EditorView {
            fn register_motions(&self, el: gpui_kit::Stateful<gpui_kit::Div>, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
                el $(.on_action(cx.listener(|this, _: &$action, _w, cx| this.move_cursor(Motion::$motion, $extend, cx))))*
            }
        }
    };
}

motion_handlers! {
    MoveLeft => Left, false;
    MoveRight => Right, false;
    MoveUp => Up, false;
    MoveDown => Down, false;
    MoveWordLeft => WordLeft, false;
    MoveWordRight => WordRight, false;
    MoveHome => Home, false;
    MoveEnd => End, false;
    MoveDocStart => DocStart, false;
    MoveDocEnd => DocEnd, false;
    MovePageUp => PageUp, false;
    MovePageDown => PageDown, false;
    SelectLeft => Left, true;
    SelectRight => Right, true;
    SelectUp => Up, true;
    SelectDown => Down, true;
    SelectWordLeft => WordLeft, true;
    SelectWordRight => WordRight, true;
    SelectHome => Home, true;
    SelectEnd => End, true;
    SelectDocStart => DocStart, true;
    SelectDocEnd => DocEnd, true;
    SelectPageUp => PageUp, true;
    SelectPageDown => PageDown, true;
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<crate::theme::ForgeTheme>().0.clone();
        let root = div()
            .id("forge-editor")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.bg)
            .cursor(CursorStyle::IBeam);
        let root = self.register_motions(root, cx);
        root.on_action(cx.listener(|this, _: &Backspace, _w, cx| this.backspace_impl(false, cx)))
            .on_action(cx.listener(|this, _: &DeleteWordLeft, _w, cx| this.backspace_impl(true, cx)))
            .on_action(cx.listener(|this, _: &Delete, _w, cx| this.delete_impl(false, cx)))
            .on_action(cx.listener(|this, _: &DeleteWordRight, _w, cx| this.delete_impl(true, cx)))
            .on_action(cx.listener(|this, _: &Newline, _w, cx| this.newline_impl(cx)))
            .on_action(cx.listener(|this, _: &Tab, _w, cx| this.tab_impl(false, cx)))
            .on_action(cx.listener(|this, _: &Untab, _w, cx| this.tab_impl(true, cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _w, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &Copy, _w, cx| this.copy_or_cut(false, cx)))
            .on_action(cx.listener(|this, _: &Cut, _w, cx| this.copy_or_cut(true, cx)))
            .on_action(cx.listener(|this, _: &Paste, _w, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &Undo, _w, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &Redo, _w, cx| this.redo(cx)))
            .on_action(cx.listener(|this, _: &ToggleComment, _w, cx| this.toggle_comment(cx)))
            .on_action(cx.listener(|this, _: &Fold, _w, cx| {
                let line = this.cursor().line;
                this.fold(line, cx)
            }))
            .on_action(cx.listener(|this, _: &Unfold, _w, cx| {
                let line = this.cursor().line;
                this.unfold(line, cx)
            }))
            .on_action(cx.listener(|this, _: &FoldAll, _w, cx| this.fold_all(cx)))
            .on_action(cx.listener(|this, _: &UnfoldAll, _w, cx| this.unfold_all(cx)))
            .on_action(cx.listener(|this, _: &GoToBracket, _w, cx| this.go_to_bracket(cx)))
            .on_action(cx.listener(|this, _: &AddNextOccurrence, _w, cx| this.add_next_occurrence(cx)))
            .on_action(cx.listener(|this, _: &SelectAllOccurrences, _w, cx| this.select_all_occurrences(cx)))
            .on_action(cx.listener(|this, _: &AddCursorAbove, _w, cx| this.add_cursor_vertical(-1, cx)))
            .on_action(cx.listener(|this, _: &AddCursorBelow, _w, cx| this.add_cursor_vertical(1, cx)))
            .on_action(cx.listener(|this, _: &Cancel, _w, cx| {
                if this.multi() {
                    this.clear_extras();
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll(cx.listener(Self::on_scroll))
            .child(EditorElement::new(cx.entity()))
    }
}

#[cfg(test)]
mod tests;
