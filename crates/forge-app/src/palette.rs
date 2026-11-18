//! The command palette. One input, three modes chosen by the first character:
//!   ">"  commands      (Ctrl+Shift+P)
//!   ":"  go to line    (Ctrl+G)
//!   else quick open a file in the project (Ctrl+P)
//! It also shows lists of choices (encodings, recent folders...), asks for a
//! line of text, and records a new shortcut. Arrow keys move, Enter picks,
//! Escape closes. Replaces `src/renderer/palette.ts`.

use crate::theme::ForgeTheme;
use crate::workspace::Workspace;
use forge_core::fuzzy::fuzzy_filter;
use forge_sys::files::ProjectFile;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, prelude::FluentBuilder, px, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, StyledText, Subscription, Window,
};
use std::ops::Range;
use std::rc::Rc;

/// What picking an item does (run on the workspace).
pub type Action = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;
/// What to do with typed text (prompt mode) or a pressed shortcut (capture mode).
pub type TextAction = Rc<dyn Fn(&mut Workspace, String, &mut Window, &mut Context<Workspace>)>;
/// Search mode: look for (query, generation); the workspace answers with `set_search_results`.
pub type SearchAction = Rc<dyn Fn(&mut Workspace, String, u64, &mut Window, &mut Context<Workspace>)>;

const MAX_ITEMS: usize = 60;

/// One entry of a list of choices.
#[derive(Clone)]
pub struct Choice {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub hint: Option<SharedString>,
    pub run: Action,
}

impl Choice {
    pub fn new(
        label: impl Into<SharedString>,
        run: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> Self {
        Self { label: label.into(), detail: None, hint: None, run: Rc::new(run) }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// A command as the palette lists it.
#[derive(Clone)]
pub struct CommandItem {
    pub id: &'static str,
    pub title: &'static str,
    pub key: Option<String>,
}

enum Mode {
    Quick,
    Choose { prompt: SharedString, choices: Vec<Choice> },
    Prompt { prompt: SharedString, verb: SharedString, submit: TextAction },
    Capture { prompt: SharedString, on_key: TextAction },
    Search { prompt: SharedString, provider: SearchAction },
}

struct Item {
    label: SharedString,
    positions: Vec<usize>,
    detail: Option<SharedString>,
    hint: Option<SharedString>,
    run: Option<Action>,
}

impl Item {
    fn note(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), positions: Vec::new(), detail: None, hint: None, run: None }
    }
}

pub enum PaletteEvent {
    /// An item was picked: run this on the workspace.
    Run(Action),
    Closed,
}

impl EventEmitter<PaletteEvent> for Palette {}

pub struct Palette {
    input: Entity<InputState>,
    mode: Mode,
    open: bool,
    items: Vec<Item>,
    selected: usize,
    commands: Vec<CommandItem>,
    files: Vec<ProjectFile>,
    line_count: Option<usize>,
    scroll: ScrollHandle,
    return_focus: Option<FocusHandle>,
    search_generation: u64,
    search_task: Option<gpui_kit::Task<()>>,
    _sub: Subscription,
}

impl Palette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _window, cx| {
            if let InputEvent::Change = event {
                this.update(cx);
            }
        });
        Self {
            input,
            mode: Mode::Quick,
            open: false,
            items: Vec::new(),
            selected: 0,
            commands: Vec::new(),
            files: Vec::new(),
            line_count: None,
            scroll: ScrollHandle::new(),
            return_focus: None,
            search_generation: 0,
            search_task: None,
            _sub: sub,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Waiting for a shortcut to be pressed (every key goes to the palette).
    pub fn capturing(&self) -> bool {
        self.open && matches!(self.mode, Mode::Capture { .. })
    }

    fn show(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            self.return_focus = window.focused(cx);
        }
        self.open = true;
        self.input.update(cx, |s, cx| {
            s.set_value(value.to_owned(), window, cx);
            s.focus(window, cx);
        });
        self.update(cx);
    }

    /// Quick open: '>' commands, ':' go to line, '' files. The workspace passes
    /// what's available now (commands that can run, the line count of the open file).
    pub fn open_quick(
        &mut self,
        prefix: &str,
        commands: Vec<CommandItem>,
        line_count: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::Quick;
        self.commands = commands;
        self.line_count = line_count;
        self.show(prefix, window, cx);
    }

    /// The project's files arrived (they're listed in the background).
    pub fn set_files(&mut self, files: Vec<ProjectFile>, cx: &mut Context<Self>) {
        self.files = files;
        if self.open && matches!(self.mode, Mode::Quick) {
            self.update(cx);
        }
    }

    /// Show a list of choices; picking one runs it.
    pub fn choose(
        &mut self,
        prompt: impl Into<SharedString>,
        choices: Vec<Choice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::Choose { prompt: prompt.into(), choices };
        self.show("", window, cx);
    }

    /// Ask for a line of text; Enter calls `submit` with it.
    pub fn prompt(
        &mut self,
        prompt: impl Into<SharedString>,
        initial: &str,
        verb: impl Into<SharedString>,
        submit: impl Fn(&mut Workspace, String, &mut Window, &mut Context<Workspace>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::Prompt { prompt: prompt.into(), verb: verb.into(), submit: Rc::new(submit) };
        self.show(initial, window, cx);
    }

    /// Choices that come from a search as you type (e.g. symbols across the
    /// project): the workspace searches, then calls `set_search_results`.
    pub fn search(
        &mut self,
        prompt: impl Into<SharedString>,
        initial: &str,
        provider: impl Fn(&mut Workspace, String, u64, &mut Window, &mut Context<Workspace>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::Search { prompt: prompt.into(), provider: Rc::new(provider) };
        self.show(initial, window, cx);
        self.input.update(cx, |s, cx| s.select_all(window, cx));
    }

    /// The results of search number `generation` (a later search drops them).
    pub fn set_search_results(&mut self, generation: u64, choices: Vec<Choice>, empty: &str, cx: &mut Context<Self>) {
        if !self.open || !matches!(self.mode, Mode::Search { .. }) || generation != self.search_generation {
            return;
        }
        self.items = if choices.is_empty() {
            vec![Item::note(empty.to_owned())]
        } else {
            choices
                .into_iter()
                .take(MAX_ITEMS)
                .map(|c| Item {
                    label: c.label,
                    positions: Vec::new(),
                    detail: c.detail,
                    hint: c.hint,
                    run: Some(c.run),
                })
                .collect()
        };
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    /// Wait for a shortcut to be pressed; Escape cancels.
    pub fn capture_key(
        &mut self,
        prompt: impl Into<SharedString>,
        on_key: impl Fn(&mut Workspace, String, &mut Window, &mut Context<Workspace>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::Capture { prompt: prompt.into(), on_key: Rc::new(on_key) };
        self.show("", window, cx);
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.mode = Mode::Quick;
        if let Some(f) = self.return_focus.take() {
            window.focus(&f, cx);
        }
        cx.emit(PaletteEvent::Closed);
        cx.notify();
    }

    /// Keys the palette handles itself while open. Returns true if it used the key.
    pub fn handle_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.open {
            return false;
        }
        if let Mode::Capture { on_key, .. } = &self.mode {
            if key.is_empty() {
                return true;
            }
            if key == "Escape" {
                self.close(window, cx);
                return true;
            }
            let on_key = on_key.clone();
            let key = key.to_owned();
            self.mode = Mode::Quick;
            self.open = false;
            if let Some(f) = self.return_focus.take() {
                window.focus(&f, cx);
            }
            cx.emit(PaletteEvent::Run(Rc::new(move |ws, w, cx| on_key(ws, key.clone(), w, cx))));
            cx.notify();
            return true;
        }
        match key {
            "Escape" => self.close(window, cx),
            "ArrowDown" | "ArrowUp" => {
                if !self.items.is_empty() {
                    let n = self.items.len();
                    self.selected =
                        if key == "ArrowDown" { (self.selected + 1) % n } else { (self.selected + n - 1) % n };
                    self.scroll.scroll_to_item(self.selected);
                    cx.notify();
                }
            }
            "PageDown" | "PageUp" => {
                if !self.items.is_empty() {
                    let n = self.items.len();
                    self.selected = if key == "PageDown" {
                        (self.selected + 10).min(n - 1)
                    } else {
                        self.selected.saturating_sub(10)
                    };
                    self.scroll.scroll_to_item(self.selected);
                    cx.notify();
                }
            }
            "Enter" => self.pick(self.selected, window, cx),
            _ => return false,
        }
        true
    }

    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.items.get(index).and_then(|i| i.run.clone()) else { return };
        self.open = false;
        self.mode = Mode::Quick;
        if let Some(f) = self.return_focus.take() {
            window.focus(&f, cx);
        }
        cx.emit(PaletteEvent::Run(run));
        cx.notify();
    }

    fn update(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        let query = value.trim();
        if let Mode::Search { provider, .. } = &self.mode {
            // Ask a moment after typing stops; only the latest search's results show.
            let provider = provider.clone();
            self.search_generation += 1;
            let generation = self.search_generation;
            self.search_task = None;
            self.items = if query.is_empty() {
                vec![Item::note("Type a name to search for")]
            } else {
                let q = query.to_owned();
                self.search_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(180)).await;
                    this.update(cx, |p, cx| {
                        if p.open && p.search_generation == generation {
                            cx.emit(PaletteEvent::Run(Rc::new(move |ws, w, cx| {
                                provider(ws, q.clone(), generation, w, cx)
                            })));
                        }
                    })
                    .ok();
                }));
                vec![Item::note("Searching…")]
            };
            self.selected = 0;
            self.scroll.scroll_to_item(0);
            cx.notify();
            return;
        }
        self.items = match &self.mode {
            Mode::Search { .. } => Vec::new(),
            Mode::Capture { .. } => vec![Item::note("Press the new shortcut now (Escape cancels)")],
            Mode::Prompt { verb, submit, .. } => {
                if query.is_empty() {
                    vec![Item::note("Type here and press Enter")]
                } else {
                    let text = query.to_owned();
                    let submit = submit.clone();
                    vec![Item {
                        label: format!("{verb} \"{query}\"").into(),
                        positions: Vec::new(),
                        detail: None,
                        hint: None,
                        run: Some(Rc::new(move |ws, w, cx| submit(ws, text.clone(), w, cx))),
                    }]
                }
            }
            Mode::Choose { choices, .. } => fuzzy_filter(query, choices, |c| c.label.as_ref(), MAX_ITEMS)
                .into_iter()
                .map(|r| Item {
                    label: r.item.label,
                    positions: r.matched.positions,
                    detail: r.item.detail,
                    hint: r.item.hint,
                    run: Some(r.item.run),
                })
                .collect(),
            Mode::Quick => {
                if let Some(q) = value.strip_prefix('>') {
                    self.command_items(q.trim())
                } else if let Some(q) = value.strip_prefix(':') {
                    self.line_items(q.trim())
                } else {
                    self.file_items(query)
                }
            }
        };
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn command_items(&self, query: &str) -> Vec<Item> {
        fuzzy_filter(query, &self.commands, |c| c.title, MAX_ITEMS)
            .into_iter()
            .map(|r| {
                let id = r.item.id;
                Item {
                    label: r.item.title.into(),
                    positions: r.matched.positions,
                    detail: None,
                    hint: r.item.key.map(Into::into),
                    run: Some(Rc::new(move |ws, w, cx| ws.run_command(id, w, cx))),
                }
            })
            .collect()
    }

    fn file_items(&self, query: &str) -> Vec<Item> {
        fuzzy_filter(query, &self.files, |f| f.rel.as_str(), MAX_ITEMS)
            .into_iter()
            .map(|r| {
                let rel: Vec<char> = r.item.rel.chars().collect();
                let slash = rel.iter().rposition(|c| *c == '/');
                let name_start = slash.map_or(0, |s| s + 1);
                let name: String = rel[name_start..].iter().collect();
                let folder: Option<String> = slash.filter(|s| *s > 0).map(|s| rel[..s].iter().collect());
                let path = r.item.path.clone();
                Item {
                    label: name.into(),
                    positions: r
                        .matched
                        .positions
                        .iter()
                        .filter(|p| **p >= name_start)
                        .map(|p| p - name_start)
                        .collect(),
                    detail: folder.map(Into::into),
                    hint: None,
                    run: Some(Rc::new(move |ws, w, cx| ws.open_path(path.clone(), w, cx))),
                }
            })
            .collect()
    }

    fn line_items(&self, query: &str) -> Vec<Item> {
        let Some(total) = self.line_count else { return vec![Item::note("Open a file first")] };
        let Ok(n) = query.parse::<i64>() else {
            return vec![Item::note(format!("Type a line number between 1 and {total}"))];
        };
        let line = n.clamp(1, total as i64) as usize;
        vec![Item {
            label: format!("Go to line {line}").into(),
            positions: Vec::new(),
            detail: None,
            hint: Some(format!("of {total}").into()),
            run: Some(Rc::new(move |ws, w, cx| ws.go_to_line(line - 1, w, cx))),
        }]
    }

    fn placeholder(&self) -> SharedString {
        match &self.mode {
            Mode::Quick => "Search files by name (type > for commands, : for a line number)".into(),
            Mode::Choose { prompt, .. }
            | Mode::Prompt { prompt, .. }
            | Mode::Capture { prompt, .. }
            | Mode::Search { prompt, .. } => prompt.clone(),
        }
    }
}

/// Byte ranges of the chars at `positions` (merged into runs).
fn char_ranges(text: &str, positions: &[usize]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (ci, (bi, ch)) in text.char_indices().enumerate() {
        if positions.contains(&ci) {
            let end = bi + ch.len_utf8();
            match out.last_mut() {
                Some(r) if r.end == bi => r.end = end,
                _ => out.push(bi..end),
            }
        }
    }
    out
}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("palette-closed");
        }
        let t = cx.global::<ForgeTheme>().0.clone();
        let placeholder = self.placeholder();
        self.input.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        let quick_commands = matches!(self.mode, Mode::Quick) && self.input.read(cx).value().starts_with('>');
        let rows = self.items.iter().enumerate().map(|(i, item)| {
            let label = StyledText::new(item.label.clone()).with_highlights(
                char_ranges(&item.label, &item.positions).into_iter().map(|r| {
                    (
                        r,
                        HighlightStyle {
                            color: Some(t.accent),
                            font_weight: Some(FontWeight::SEMIBOLD),
                            ..Default::default()
                        },
                    )
                }),
            );
            div()
                .id(("palette-item", i))
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(t.corner(px(2.0)))
                .whitespace_nowrap()
                .cursor_pointer()
                .when(i == self.selected, |d| d.bg(t.bg_active))
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.selected != i {
                        this.selected = i;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| this.pick(i, window, cx)))
                .child(div().flex_none().child(label))
                .when_some(item.detail.clone(), |d, detail| {
                    d.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(t.fg_muted)
                            .text_size(px(12.0))
                            .child(detail),
                    )
                })
                .when_some(item.hint.clone(), |d, hint| {
                    d.child(
                        div()
                            .ml_auto()
                            .px(px(6.0))
                            .py(px(1.0))
                            .border_1()
                            .border_color(t.border)
                            .rounded(t.corner(px(2.0)))
                            .text_color(t.fg_muted)
                            .text_size(px(11.0))
                            .child(hint),
                    )
                })
        });
        let empty_text = match &self.mode {
            Mode::Quick if quick_commands => Some("No matching commands"),
            Mode::Quick => Some("No matching files"),
            // An empty list of choices: the prompt says why.
            Mode::Choose { choices, .. } if !choices.is_empty() => Some("Nothing matches"),
            _ => None,
        };
        let empty = empty_text
            .filter(|_| self.items.is_empty())
            .map(|text| div().px(px(12.0)).py(px(10.0)).text_color(t.fg_muted).child(text));
        div()
            .id("palette-overlay")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(48.0))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close(window, cx)))
            .child(
                div()
                    .id("palette")
                    .relative()
                    .w(px(620.0))
                    .max_w_full()
                    .bg(t.bg_panel)
                    .border_1()
                    .border_color(t.border)
                    .rounded(t.corner(px(2.0)))
                    .shadow_lg()
                    .overflow_hidden()
                    .font_family(crate::theme::UI_FONT)
                    .text_size(px(13.0))
                    .text_color(t.fg)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(corner_mark(t.heat_3))
                    .child(
                        div()
                            .border_b_1()
                            .border_color(t.border)
                            .px(px(6.0))
                            .py(px(4.0))
                            .child(Input::new(&self.input).appearance(false).cleanable(false)),
                    )
                    .child(
                        div()
                            .id("palette-list")
                            .max_h(px(420.0))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .p(px(4.0))
                            .children(rows)
                            .children(empty),
                    ),
            )
    }
}

/// The small ember "L" in the top-left corner of panels and dialogs.
pub fn corner_mark(color: gpui_kit::Hsla) -> impl IntoElement {
    div().absolute().left_0().top_0().w(px(7.0)).h(px(7.0)).border_l_2().border_t_2().border_color(color)
}

#[cfg(test)]
mod tests {
    use super::char_ranges;

    #[test]
    fn highlight_ranges_are_byte_ranges_merged_into_runs() {
        assert_eq!(char_ranges("Save File", &[0, 1, 5]), vec![0..2, 5..6]);
        assert_eq!(char_ranges("é_x", &[0, 2]), vec![0..2, 3..4]);
    }
}
