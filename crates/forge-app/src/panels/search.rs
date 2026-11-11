//! The Search panel (Ctrl+Shift+F): search the whole project; results grouped
//! by file, clicking one opens the file with the match selected. Replaces
//! `src/renderer/searchView.ts`.

use crate::find_bar::FindOption;
use crate::theme::{ForgeTheme, Theme};
use forge_sys::search::{search_project, SearchLimits, SearchResult};
use forge_text::find::FindOptions;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, prelude::FluentBuilder, AnyElement, App, AppContext, Context, Entity, EventEmitter, Focusable, FontWeight,
    HighlightStyle, InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, StyledText, Subscription, Task, Window,
};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub enum SearchEvent {
    /// Open a match: file, line, column, length (chars).
    OpenMatch(PathBuf, usize, usize, usize),
}

impl EventEmitter<SearchEvent> for SearchPanel {}

pub struct SearchPanel {
    input: Entity<InputState>,
    opts: FindOptions,
    root: Option<PathBuf>,
    result: Option<SearchResult>,
    status: String,
    /// Results for older text stay until new ones arrive (shown dimmed).
    stale: bool,
    collapsed: HashSet<String>,
    generation: Arc<AtomicU64>,
    task: Option<Task<()>>,
    _sub: Subscription,
}

impl SearchPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search in project"));
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, _, cx| match e {
            InputEvent::Change => {
                this.stale = true;
                this.schedule(Duration::from_millis(250), cx);
            }
            InputEvent::PressEnter { .. } => this.schedule(Duration::ZERO, cx),
            _ => {}
        });
        Self {
            input,
            opts: FindOptions::default(),
            root: None,
            result: None,
            status: String::new(),
            stale: false,
            collapsed: HashSet::new(),
            generation: Arc::new(AtomicU64::new(0)),
            task: None,
            _sub: sub,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.root = root;
        self.result = None;
        self.status.clear();
        cx.notify();
    }

    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.input.focus_handle(cx).is_focused(window)
    }

    /// Focus the search box, optionally starting with some text (e.g. the editor selection).
    pub fn focus(&mut self, initial: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = initial.filter(|t| !t.is_empty() && !t.contains('\n')) {
            self.input.update(cx, |s, cx| s.set_value(text, window, cx));
            self.schedule(Duration::ZERO, cx);
        }
        self.input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
    }

    /// Search again (e.g. after files were saved).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.input.read(cx).value().is_empty() {
            self.schedule(Duration::ZERO, cx);
        }
    }

    pub fn toggle(&mut self, option: FindOption, cx: &mut Context<Self>) {
        match option {
            FindOption::Case => self.opts.case_sensitive = !self.opts.case_sensitive,
            FindOption::Word => self.opts.whole_word = !self.opts.whole_word,
            FindOption::Regex => self.opts.regex = !self.opts.regex,
        }
        self.schedule(Duration::ZERO, cx);
    }

    fn schedule(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let query = self.input.read(cx).value().to_string();
        let Some(root) = self.root.clone() else {
            self.status = "Open a folder to search it.".into();
            self.result = None;
            cx.notify();
            return;
        };
        if query.is_empty() {
            self.status.clear();
            self.result = None;
            self.stale = false;
            cx.notify();
            return;
        }
        let opts = self.opts;
        let counter = self.generation.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            if counter.load(Ordering::SeqCst) != generation {
                return;
            }
            this.update(cx, |p, cx| {
                p.status = "Searching…".into();
                cx.notify();
            })
            .ok();
            let c2 = counter.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    search_project(&root, &query, opts, SearchLimits::default(), &|| {
                        c2.load(Ordering::SeqCst) != generation
                    })
                })
                .await;
            this.update(cx, |p, cx| {
                // A newer search started meanwhile.
                if counter.load(Ordering::SeqCst) == generation {
                    p.show(result);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn show(&mut self, r: SearchResult) {
        self.stale = false;
        self.status = if let Some(e) = &r.error {
            format!("Invalid regex: {e}")
        } else if r.total == 0 {
            "No results".into()
        } else {
            format!(
                "{}{} {} in {} {}{}",
                r.total,
                if r.truncated { "+" } else { "" },
                if r.total == 1 { "result" } else { "results" },
                r.files.len(),
                if r.files.len() == 1 { "file" } else { "files" },
                if r.truncated { " (stopped early, narrow the search)" } else { "" }
            )
        };
        if !r.skipped.is_empty() {
            let n = r.skipped.len();
            self.status.push_str(&format!(" · {n} file{} over 20 MB not searched", if n > 1 { "s" } else { "" }));
        }
        self.result = Some(r);
    }

    fn toggle_button(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        which: FindOption,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .px(t.px(5.0))
            .h(t.px(22.0))
            .flex()
            .items_center()
            .rounded(t.px(2.0))
            .text_size(t.px(12.0))
            .cursor_pointer()
            .when(on, |d| d.bg(t.bg_active).text_color(t.heat_2))
            .when(!on, |d| d.text_color(t.fg_muted).hover(|s| s.bg(t.bg_hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(which, cx)))
            .child(label)
            .into_any_element()
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(t.px(4.0))
            .px(t.px(8.0))
            .py(t.px(6.0))
            .child(
                div()
                    .flex_1()
                    .border_1()
                    .border_color(if self.result.as_ref().is_some_and(|r| r.error.is_some()) {
                        t.err
                    } else {
                        t.line_strong
                    })
                    .child(Input::new(&self.input).appearance(false).cleanable(false).h(t.px(24.0))),
            )
            .child(self.toggle_button("search-case", "Aa", self.opts.case_sensitive, FindOption::Case, &t, cx))
            .child(self.toggle_button("search-word", "ab", self.opts.whole_word, FindOption::Word, &t, cx))
            .child(self.toggle_button("search-regex", ".*", self.opts.regex, FindOption::Regex, &t, cx));
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(r) = &self.result {
            for (fi, file) in r.files.iter().enumerate() {
                let collapsed = self.collapsed.contains(&file.rel);
                let (dir, name) = match file.rel.rfind('/') {
                    Some(i) => (file.rel[..i].to_owned(), file.rel[i + 1..].to_owned()),
                    None => (String::new(), file.rel.clone()),
                };
                let rel = file.rel.clone();
                rows.push(
                    div()
                        .id(("search-file", fi))
                        .flex()
                        .items_center()
                        .gap(t.px(6.0))
                        .px(t.px(8.0))
                        .py(t.px(2.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.bg_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.collapsed.remove(&rel) {
                                this.collapsed.insert(rel.clone());
                            }
                            cx.notify();
                        }))
                        .child(div().w(t.px(10.0)).text_color(t.fg_muted).child(if collapsed { "▸" } else { "▾" }))
                        .child(div().font_weight(FontWeight::MEDIUM).child(name))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(t.px(11.0))
                                .text_color(t.fg_muted)
                                .child(dir),
                        )
                        .child(div().text_size(t.px(11.0)).text_color(t.fg_muted).child(file.matches.len().to_string()))
                        .into_any_element(),
                );
                if collapsed {
                    continue;
                }
                for (mi, m) in file.matches.iter().enumerate() {
                    let text = m.preview.text.clone();
                    let start = text.char_indices().nth(m.preview.start).map_or(text.len(), |(b, _)| b);
                    let end = text.char_indices().nth(m.preview.start + m.length).map_or(text.len(), |(b, _)| b);
                    let styled = StyledText::new(text).with_highlights([(
                        start..end,
                        HighlightStyle { background_color: Some(t.match_bg), color: Some(t.fg), ..Default::default() },
                    )]);
                    let (path, line, col, len) = (file.path.clone(), m.line, m.col, m.length);
                    rows.push(
                        div()
                            .id(SharedString::from(format!("search-match-{fi}-{mi}")))
                            .flex()
                            .gap(t.px(8.0))
                            .pl(t.px(24.0))
                            .pr(t.px(8.0))
                            .py(t.px(1.0))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.bg_hover))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(SearchEvent::OpenMatch(path.clone(), line, col, len))
                            }))
                            .child(
                                div()
                                    .min_w(t.px(28.0))
                                    .text_size(t.px(11.0))
                                    .text_color(t.fg_muted)
                                    .child((m.line + 1).to_string()),
                            )
                            .child(div().text_color(t.fg_muted).child(styled))
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_size(t.px(13.0))
            .text_color(t.fg)
            .child(bar)
            .when(!self.status.is_empty(), |d| {
                d.child(
                    div()
                        .px(t.px(10.0))
                        .pb(t.px(4.0))
                        .text_size(t.px(12.0))
                        .text_color(t.fg_muted)
                        .child(self.status.clone()),
                )
            })
            .child(
                div()
                    .id("search-results")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(self.stale, |d| d.opacity(0.6))
                    .children(rows),
            )
    }
}
