//! The find/replace bar (Ctrl+F / Ctrl+H), floating over the top-right of the
//! editor. Every match in the file is highlighted; the current one is
//! selected. Enter / Shift+Enter go to the next / previous match. Replaces
//! `src/renderer/findBar.ts`.

use crate::editor::EditorView;
use crate::theme::{ForgeTheme, Theme};
use forge_text::find::{
    find_matches, match_index_from, replace_all, replacement_for, FindOptions, Match, DEFAULT_LIMIT,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, AppContext, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, WeakEntity, Window,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindOption {
    Case,
    Word,
    Regex,
}

pub struct FindBar {
    find: Entity<InputState>,
    replace: Entity<InputState>,
    open: bool,
    show_replace: bool,
    opts: FindOptions,
    matches: Vec<Match>,
    current: Option<usize>,
    error: Option<String>,
    editor: Option<WeakEntity<EditorView>>,
    _subs: Vec<Subscription>,
}

impl FindBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replace = cx.new(|cx| InputState::new(window, cx).placeholder("Replace"));
        let subs = vec![
            cx.subscribe_in(&find, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::Change => this.search(true, window, cx),
                InputEvent::PressEnter { shift, .. } => this.go(!*shift, window, cx),
                _ => {}
            }),
            cx.subscribe_in(&replace, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { secondary, .. } = e {
                    if *secondary {
                        this.replace_all(window, cx);
                    } else {
                        this.replace_one(window, cx);
                    }
                }
            }),
        ];
        Self {
            find,
            replace,
            open: false,
            show_replace: false,
            opts: FindOptions::default(),
            matches: Vec::new(),
            current: None,
            error: None,
            editor: None,
            _subs: subs,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Keyboard focus is in the bar.
    pub fn has_focus(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.open && (self.find.focus_handle(cx).is_focused(window) || self.replace.focus_handle(cx).is_focused(window))
    }

    /// Open the bar on `editor` (with the replace row if `replace`), prefilled with the selected text.
    pub fn open(&mut self, editor: &Entity<EditorView>, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selected = editor.read(cx).selected_text(cx);
        // Prefill only when opening: with the bar open, the selection is usually just the current match.
        if !self.open && !selected.is_empty() && !selected.contains('\n') {
            let text = if self.opts.regex { regex::escape(&selected) } else { selected };
            self.find.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.switch_editor(Some(editor.downgrade()), cx);
        self.open = true;
        if replace {
            self.show_replace = true;
        }
        let focus_replace = replace && !self.find.read(cx).value().is_empty();
        let input = if focus_replace { self.replace.clone() } else { self.find.clone() };
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        self.search(false, window, cx);
    }

    /// The bar now acts on another editor (the other side of a split was focused).
    pub fn switch_editor(&mut self, editor: Option<WeakEntity<EditorView>>, cx: &mut Context<Self>) {
        if self.editor.as_ref().map(|e| e.entity_id()) == editor.as_ref().map(|e| e.entity_id()) {
            return;
        }
        if let Some(old) = self.editor.take().and_then(|e| e.upgrade()) {
            old.update(cx, |e, cx| e.set_matches(Vec::new(), cx));
        }
        self.editor = editor;
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.matches.clear();
        if let Some(ed) = self.editor.as_ref().and_then(|e| e.upgrade()) {
            ed.update(cx, |e, cx| e.set_matches(Vec::new(), cx));
            window.focus(&ed.focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// The document changed (an edit or a tab switch): refresh the highlights without jumping.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.search(false, window, cx);
        }
    }

    fn editor(&self) -> Option<Entity<EditorView>> {
        self.editor.as_ref().and_then(|e| e.upgrade())
    }

    fn search(&mut self, jump: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = self.editor() else { return };
        let query = self.find.read(cx).value().to_string();
        let (matches, error) = match ed.read(cx).document() {
            Some(doc) => {
                let r = find_matches(doc.read(cx).doc.buffer.lines(), &query, self.opts, DEFAULT_LIMIT);
                (r.matches, r.error)
            }
            None => (Vec::new(), None),
        };
        self.matches = matches;
        self.error = error;
        let sel = ed.read(cx).selection();
        let start = if sel.anchor <= sel.head { sel.anchor } else { sel.head };
        self.current = match_index_from(&self.matches, start, true);
        let shown = self.matches.clone();
        ed.update(cx, |e, cx| e.set_matches(shown, cx));
        if jump {
            self.select_current(cx);
        }
        let _ = window;
        cx.notify();
    }

    fn is_selected(&self, m: &Match, cx: &gpui_kit::App) -> bool {
        self.editor().is_some_and(|ed| {
            let s = ed.read(cx).selection();
            s.anchor == m.start && s.head == m.end
        })
    }

    fn select_current(&mut self, cx: &mut Context<Self>) {
        let Some(m) = self.current.and_then(|i| self.matches.get(i)).copied() else { return };
        if let Some(ed) = self.editor() {
            ed.update(cx, |e, cx| e.select(m.start, m.end, cx));
        }
    }

    fn go(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        let on_match = self.current.is_some_and(|i| self.is_selected(&self.matches[i], cx));
        if on_match {
            let i = self.current.unwrap_or(0);
            self.current = Some(if forward { (i + 1) % n } else { (i + n - 1) % n });
        } else if let Some(ed) = self.editor() {
            let head = ed.read(cx).selection().head;
            self.current = match_index_from(&self.matches, head, forward);
        }
        self.select_current(cx);
        let _ = window;
        cx.notify();
    }

    fn replace_one(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.current.and_then(|i| self.matches.get(i)).copied() else { return };
        if !self.is_selected(&m, cx) {
            // The first press selects the match, the second replaces it.
            self.select_current(cx);
            cx.notify();
            return;
        }
        let (query, with, opts) =
            (self.find.read(cx).value().to_string(), self.replace.read(cx).value().to_string(), self.opts);
        if let Some(ed) = self.editor() {
            ed.update(cx, |e, cx| {
                e.run_edit(cx, |b| {
                    let text = replacement_for(&b.range(m.start, m.end), &query, &with, opts);
                    b.delete(m.start, m.end);
                    b.insert(m.start, &text)
                })
            });
        }
        self.search(false, window, cx);
        self.select_current(cx);
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let matches = self.matches.clone();
        let (query, with, opts) =
            (self.find.read(cx).value().to_string(), self.replace.read(cx).value().to_string(), self.opts);
        if let Some(ed) = self.editor() {
            ed.update(cx, |e, cx| e.run_edit(cx, |b| replace_all(b, &matches, &with, &query, opts)));
        }
        self.search(false, window, cx);
    }

    pub fn toggle(&mut self, option: FindOption, window: &mut Window, cx: &mut Context<Self>) {
        match option {
            FindOption::Case => self.opts.case_sensitive = !self.opts.case_sensitive,
            FindOption::Word => self.opts.whole_word = !self.opts.whole_word,
            FindOption::Regex => self.opts.regex = !self.opts.regex,
        }
        self.search(true, window, cx);
    }

    fn count_text(&self, cx: &gpui_kit::App) -> String {
        if self.find.read(cx).value().is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            return "No results".into();
        }
        let shown = match self.current {
            Some(i) if self.is_selected(&self.matches[i], cx) => (i + 1).to_string(),
            _ => "?".into(),
        };
        let total =
            if self.matches.len() >= DEFAULT_LIMIT { "10000+".to_owned() } else { self.matches.len().to_string() };
        format!("{shown} of {total}")
    }

    fn button(&self, id: &'static str, label: &'static str, on: bool, t: &Theme) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .px(px(5.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .rounded(px(2.0))
            .text_size(px(12.0))
            .cursor_pointer()
            .when(on, |d| d.bg(t.bg_active).text_color(t.heat_2))
            .when(!on, |d| d.text_color(t.fg_muted).hover(|s| s.bg(t.bg_hover).text_color(t.fg)))
            .child(label)
    }
}

impl Render for FindBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("find-bar-closed").into_any_element();
        }
        let t = cx.global::<ForgeTheme>().0.clone();
        let count = self.count_text(cx);
        let none = !count.is_empty() && self.matches.is_empty();
        let opt = |this: &Self, id, label, on, which: FindOption, cx: &mut Context<Self>| -> AnyElement {
            this.button(id, label, on, &t)
                .on_click(cx.listener(move |this, _, window, cx| this.toggle(which, window, cx)))
                .into_any_element()
        };
        let find_row = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .w(px(220.0))
                    .border_1()
                    .border_color(if self.error.is_some() { t.err } else { t.line_strong })
                    .child(Input::new(&self.find).appearance(false).cleanable(false).h(px(24.0))),
            )
            .child(opt(self, "find-case", "Aa", self.opts.case_sensitive, FindOption::Case, cx))
            .child(opt(self, "find-word", "ab", self.opts.whole_word, FindOption::Word, cx))
            .child(opt(self, "find-regex", ".*", self.opts.regex, FindOption::Regex, cx))
            .child(
                div()
                    .min_w(px(64.0))
                    .text_size(px(12.0))
                    .text_color(if none { t.err } else { t.fg_muted })
                    .child(SharedString::from(count)),
            )
            .child(
                self.button("find-prev", "↑", false, &t).on_click(cx.listener(|this, _, w, cx| this.go(false, w, cx))),
            )
            .child(
                self.button("find-next", "↓", false, &t).on_click(cx.listener(|this, _, w, cx| this.go(true, w, cx))),
            )
            .child(self.button("find-toggle-replace", "⇄", self.show_replace, &t).on_click(cx.listener(
                |this, _, window, cx| {
                    this.show_replace = !this.show_replace;
                    let input = if this.show_replace { this.replace.clone() } else { this.find.clone() };
                    input.update(cx, |s, cx| s.focus(window, cx));
                    cx.notify();
                },
            )))
            .child(self.button("find-close", "×", false, &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))));
        let replace_row = self.show_replace.then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .w(px(220.0))
                        .border_1()
                        .border_color(t.line_strong)
                        .child(Input::new(&self.replace).appearance(false).cleanable(false).h(px(24.0))),
                )
                .child(
                    self.button("find-replace", "Replace", false, &t)
                        .on_click(cx.listener(|this, _, w, cx| this.replace_one(w, cx))),
                )
                .child(
                    self.button("find-replace-all", "All", false, &t)
                        .on_click(cx.listener(|this, _, w, cx| this.replace_all(w, cx))),
                )
        });
        div()
            .id("find-bar")
            .absolute()
            .top(px(4.0))
            .right(px(18.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(6.0))
            .bg(t.bg_panel)
            .border_1()
            .border_color(t.border)
            .shadow_md()
            .text_color(t.fg)
            .when_some(self.error.clone(), |d, e| {
                d.tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(e.clone()).build(window, cx))
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(find_row)
            .children(replace_row)
            .into_any_element()
    }
}

impl Focusable for FindBar {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.find.focus_handle(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::theme::{ForgeTheme, Theme};
    use gpui_kit::{TestAppContext, VisualTestContext};
    use std::sync::Arc;

    struct Host {
        editor: Entity<EditorView>,
        bar: Entity<FindBar>,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.editor.clone()).child(self.bar.clone())
        }
    }

    fn setup<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<EditorView>, Entity<FindBar>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            cx.set_global(ForgeTheme(Arc::new(Theme::molten())));
            crate::editor::bind_keys(cx);
        });
        let (host, cx) = cx.add_window_view(|window, cx| {
            let editor = cx.new(EditorView::new);
            let bar = cx.new(|cx| FindBar::new(window, cx));
            Host { editor, bar }
        });
        let (editor, bar) = host.read_with(cx, |h, _| (h.editor.clone(), h.bar.clone()));
        let text = text.to_owned();
        editor.update(cx, |e, cx| {
            let doc = cx.new(|_| Document::new("a.py", &text));
            e.show(doc, cx);
        });
        cx.run_until_parked();
        (editor, bar, cx)
    }

    fn text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
        editor.read_with(cx, |e, cx| e.document().map(|d| d.read(cx).doc.text()).unwrap_or_default())
    }

    #[gpui_kit::test]
    fn typing_finds_every_match_and_options_narrow_them(cx: &mut TestAppContext) {
        let (editor, bar, cx) = setup(cx, "cat dog cat\nCat");
        bar.update_in(cx, |b, window, cx| b.open(&editor, false, window, cx));
        cx.simulate_input("cat");
        cx.run_until_parked();
        assert_eq!(bar.read_with(cx, |b, _| b.matches.len()), 3);
        // The first match is selected as you type.
        assert_eq!(editor.read_with(cx, |e, cx| e.selected_text(cx)), "cat");
        bar.update_in(cx, |b, window, cx| b.toggle(FindOption::Case, window, cx));
        assert_eq!(bar.read_with(cx, |b, _| b.matches.len()), 2);
        assert_eq!(bar.read_with(cx, |b, cx| b.count_text(cx)), "1 of 2");
        bar.update_in(cx, |b, window, cx| b.go(true, window, cx));
        assert_eq!(bar.read_with(cx, |b, cx| b.count_text(cx)), "2 of 2");
    }

    #[gpui_kit::test]
    fn replace_selects_first_then_replaces_and_replace_all_does_the_rest(cx: &mut TestAppContext) {
        let (editor, bar, cx) = setup(cx, "a1 a2 a3");
        bar.update_in(cx, |b, window, cx| {
            b.open(&editor, true, window, cx);
            b.opts.regex = true;
            b.find.update(cx, |s, cx| s.set_value(r"a(\d)", window, cx));
            b.replace.update(cx, |s, cx| s.set_value("b$1", window, cx));
            b.search(true, window, cx);
        });
        bar.update_in(cx, |b, window, cx| b.replace_one(window, cx));
        assert_eq!(text(&editor, cx), "b1 a2 a3");
        bar.update_in(cx, |b, window, cx| b.replace_all(window, cx));
        assert_eq!(text(&editor, cx), "b1 b2 b3");
        // One undo step brings back everything Replace All changed.
        editor.update(cx, |e, cx| e.undo(cx));
        assert_eq!(text(&editor, cx), "b1 a2 a3");
    }

    #[gpui_kit::test]
    fn closing_clears_the_highlights(cx: &mut TestAppContext) {
        let (editor, bar, cx) = setup(cx, "x x");
        bar.update_in(cx, |b, window, cx| b.open(&editor, false, window, cx));
        cx.simulate_input("x");
        assert_eq!(editor.read_with(cx, |e, _| e.matches.len()), 2);
        bar.update_in(cx, |b, window, cx| b.close(window, cx));
        assert_eq!(editor.read_with(cx, |e, _| e.matches.len()), 0);
        assert!(!bar.read_with(cx, |b, _| b.is_open()));
    }
}
