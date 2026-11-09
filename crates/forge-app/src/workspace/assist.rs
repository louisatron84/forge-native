//! Help while typing: the autocomplete popup (opens when you type a name or
//! '.', or on Ctrl+Space; typing more filters it), hover cards (type,
//! signature, docs and the problems at that spot), go to definition (F12,
//! Ctrl+click) and Go Back. Replaces `src/renderer/completion.ts` and the
//! hover/definition parts of `main.ts`.

use super::Workspace;
use crate::theme::{Theme, CODE_FONT};
use forge_lsp::completion::{filter_completions, hover_sections, kind_icon, word_start_at, CompletionItem, HoverSection};
use forge_lsp::navigation::{FileEdits, TextEdit};
use forge_text::Pos;
use gpui_kit::{
    anchored, deferred, div, point, prelude::FluentBuilder, px, AnyElement, Context, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, ScrollHandle, StatefulInteractiveElement, Styled, Task, Window,
};
use std::path::PathBuf;

const MAX_SHOWN: usize = 200;

#[derive(Default)]
pub(crate) struct CompletionState {
    pub open: bool,
    items: Vec<CompletionItem>,
    shown: Vec<CompletionItem>,
    selected: usize,
    incomplete: bool,
    /// Where the word being completed starts (chars), in which file and side.
    line: usize,
    start_col: usize,
    path: String,
    group: usize,
    generation: u64,
    task: Option<Task<()>>,
    resolve_task: Option<Task<()>>,
    scroll: ScrollHandle,
}

#[derive(Default)]
pub(crate) struct HoverState {
    at: Option<Point<Pixels>>,
    sections: Vec<HoverSection>,
    problems: Vec<(u8, String)>,
    generation: u64,
    task: Option<Task<()>>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Workspace {
    fn active_python_doc(&self, cx: &gpui_kit::App) -> Option<(usize, String)> {
        let g = self.tabs.active_group.min(1);
        let doc = self.editors[g].read(cx).document()?.clone();
        let d = &doc.read(cx).doc;
        (d.mode().language.server == Some("pyright")).then(|| (g, d.path().to_owned()))
    }

    // ---------- autocomplete ----------

    /// The user typed `text` in the editor.
    pub(crate) fn completion_on_type(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some((group, _)) = self.active_python_doc(cx) else { return };
        let Some(last) = text.chars().last() else { return };
        let single_word = text.chars().count() == 1 && is_word_char(last);
        if self.completion.open {
            if last == '.' {
                self.request_completion(Some('.'), cx);
            } else if single_word {
                if self.completion.incomplete {
                    self.request_completion(None, cx);
                } else {
                    self.refilter_completion(cx);
                }
            } else {
                self.close_completion(cx);
            }
            return;
        }
        if last == '.' {
            self.request_completion(Some('.'), cx);
        } else if single_word {
            // Don't pop up for numbers like "42".
            let ed = self.editors[group].read(cx);
            let Some(doc) = ed.document() else { return };
            let c = ed.cursor();
            let line = doc.read(cx).doc.buffer.line(c.line).to_owned();
            let start = word_start_at(&line, c.col);
            if !line.chars().nth(start).is_some_and(|ch| ch.is_ascii_digit()) {
                self.request_completion(None, cx);
            }
        }
    }

    /// Ctrl+Space, or typing.
    pub(crate) fn request_completion(&mut self, trigger: Option<char>, cx: &mut Context<Self>) {
        let Some((group, path)) = self.active_python_doc(cx) else { return };
        let Some(server) = self.lsp.server.clone() else { return };
        self.lsp_flush(cx);
        let ed = self.editors[group].clone();
        let Some(doc) = ed.read(cx).document().cloned() else { return };
        let pos = ed.read(cx).cursor();
        let at = self.to_lsp(&doc.read(cx).doc, pos);
        self.completion.generation += 1;
        let generation = self.completion.generation;
        self.completion.task = Some(cx.spawn(async move |this, cx| {
            let (items, incomplete) = server.completion(&path, at, trigger).await;
            this.update(cx, |ws, cx| {
                if ws.completion.generation != generation {
                    return; // something newer happened
                }
                let ed = ws.editors[group].read(cx);
                let now = ed.cursor();
                let Some(doc) = ed.document() else { return };
                if now.line != pos.line || doc.read(cx).doc.path() != path {
                    return;
                }
                let line = doc.read(cx).doc.buffer.line(now.line).to_owned();
                ws.completion.items = items;
                ws.completion.incomplete = incomplete;
                ws.completion.path = path.clone();
                ws.completion.group = group;
                ws.completion.line = now.line;
                ws.completion.start_col = word_start_at(&line, now.col);
                ws.completion.open = true;
                ws.refilter_completion(cx);
            })
            .ok();
        }));
    }

    /// Keep the list in sync with what's typed, or close it.
    pub(crate) fn refilter_completion(&mut self, cx: &mut Context<Self>) {
        if !self.completion.open {
            return;
        }
        let ed = self.editors[self.completion.group].read(cx);
        let pos = ed.cursor();
        let Some(doc) = ed.document() else { return self.close_completion(cx) };
        let d = &doc.read(cx).doc;
        if d.path() != self.completion.path || pos.line != self.completion.line || pos.col < self.completion.start_col {
            return self.close_completion(cx);
        }
        let line = d.buffer.line(pos.line).to_owned();
        if word_start_at(&line, pos.col) != self.completion.start_col {
            return self.close_completion(cx);
        }
        let prefix: String = line.chars().skip(self.completion.start_col).take(pos.col - self.completion.start_col).collect();
        let shown: Vec<CompletionItem> = filter_completions(&self.completion.items, &prefix).into_iter().take(MAX_SHOWN).collect();
        // Nothing left, or the only suggestion is exactly what's already typed: nothing to offer.
        let only_typed = shown.len() == 1 && shown[0].insert_text.as_deref().unwrap_or(&shown[0].label) == prefix;
        if shown.is_empty() || only_typed {
            return self.close_completion(cx);
        }
        self.completion.shown = shown;
        self.completion.selected = 0;
        self.completion.scroll.scroll_to_item(0);
        self.resolve_selected(cx);
        cx.notify();
    }

    pub(crate) fn close_completion(&mut self, cx: &mut Context<Self>) {
        if self.completion.open {
            self.completion.open = false;
            self.completion.generation += 1;
            self.completion.resolve_task = None;
            cx.notify();
        }
    }

    fn move_completion(&mut self, delta: i32, cx: &mut Context<Self>) {
        let n = self.completion.shown.len() as i32;
        if n == 0 {
            return;
        }
        let mut next = self.completion.selected as i32 + delta;
        next = if delta.abs() == 1 { next.rem_euclid(n) } else { next.clamp(0, n - 1) };
        self.completion.selected = next as usize;
        self.completion.scroll.scroll_to_item(self.completion.selected);
        self.resolve_selected(cx);
        cx.notify();
    }

    /// Fetch the type/docs of the selected suggestion if the server fills them in lazily.
    fn resolve_selected(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.completion.shown.get(self.completion.selected).cloned() else { return };
        if item.detail.is_some() || item.documentation.is_some() {
            return;
        }
        let Some(server) = self.lsp.server.clone() else { return };
        let generation = self.completion.generation;
        let index = self.completion.selected;
        self.completion.resolve_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(80)).await;
            let full = server.resolve_completion(item).await;
            this.update(cx, |ws, cx| {
                if ws.completion.generation == generation && ws.completion.selected == index {
                    if let Some(slot) = ws.completion.shown.get_mut(index) {
                        *slot = full;
                    }
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Insert the chosen suggestion (and its import, for an auto-import).
    fn accept_completion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.completion.shown.get(index).cloned() else { return self.close_completion(cx) };
        let group = self.completion.group;
        self.close_completion(cx);
        let ed = self.editors[group].clone();
        let Some(doc) = ed.read(cx).document().cloned() else { return };
        let pos = ed.read(cx).cursor();
        let (start, text) = {
            let d = &doc.read(cx).doc;
            let line = d.buffer.line(pos.line).to_owned();
            match &item.text_edit {
                Some((s, _, t)) => (super::lsp::from_lsp(d.buffer.line(s.line), *s), t.clone()),
                None => (Pos::new(pos.line, word_start_at(&line, pos.col)), item.insert_text.clone().unwrap_or_else(|| item.label.clone())),
            }
        };
        ed.update(cx, |e, cx| {
            e.run_edit(cx, |b| {
                b.delete(start, pos);
                b.insert(start, &text)
            })
        });
        let path = doc.read(cx).doc.path().to_owned();
        let apply_extra = move |ws: &mut Workspace, edits: Vec<TextEdit>, window: &mut Window, cx: &mut Context<Workspace>| {
            if !edits.is_empty() {
                let _ = ws.apply_file_edits(&FileEdits::from([(path.clone(), edits)]), window, cx);
            }
        };
        if !item.additional_edits.is_empty() || !item.auto_import {
            return apply_extra(self, item.additional_edits.clone(), window, cx);
        }
        // An auto-import's import line comes with the full item.
        let Some(server) = self.lsp.server.clone() else { return };
        cx.spawn_in(window, async move |this, cx| {
            let full = server.resolve_completion(item).await;
            this.update_in(cx, |ws, window, cx| apply_extra(ws, full.additional_edits, window, cx)).ok();
        })
        .detach();
    }

    /// Keys while the popup is open. True if used.
    pub(crate) fn completion_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.completion.open {
            return false;
        }
        match key {
            "ArrowDown" => self.move_completion(1, cx),
            "ArrowUp" => self.move_completion(-1, cx),
            "PageDown" => self.move_completion(8, cx),
            "PageUp" => self.move_completion(-8, cx),
            "Enter" | "Tab" => {
                let i = self.completion.selected;
                self.accept_completion(i, window, cx);
            }
            "Escape" => self.close_completion(cx),
            "ArrowLeft" | "ArrowRight" | "Home" | "End" => {
                self.close_completion(cx);
                return false; // the editor moves the cursor too
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn render_completion(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.completion.open || self.completion.shown.is_empty() {
            return None;
        }
        let ed = self.editors[self.completion.group].read(cx);
        let anchor = ed.screen_bounds(Pos::new(self.completion.line, self.completion.start_col))?;
        let rows = self.completion.shown.iter().enumerate().map(|(i, item)| {
            let (icon, _) = kind_icon(item.kind);
            div()
                .id(("completion", i))
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .h(px(22.0))
                .whitespace_nowrap()
                .cursor_pointer()
                .when(i == self.completion.selected, |d| d.bg(t.bg_active))
                .on_mouse_move(cx.listener(move |ws, _, _, cx| {
                    if ws.completion.selected != i {
                        ws.completion.selected = i;
                        ws.resolve_selected(cx);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |ws, _, window, cx| ws.accept_completion(i, window, cx)))
                .child(div().w(px(14.0)).text_color(t.accent).child(icon))
                .child(div().child(item.label.clone()))
                .when(item.auto_import, |d| d.child(div().text_size(px(11.0)).text_color(t.fg_muted).child("auto-import")))
        });
        let selected = self.completion.shown.get(self.completion.selected);
        let details = selected.filter(|s| s.detail.is_some() || s.documentation.is_some()).map(|s| {
            div()
                .w(px(320.0))
                .max_h(px(260.0))
                .p(px(8.0))
                .bg(t.bg_panel)
                .border_1()
                .border_color(t.border)
                .text_size(px(12.0))
                .overflow_hidden()
                .when_some(s.detail.clone(), |d, detail| d.child(div().font_family(CODE_FONT).text_color(t.fg).child(detail)))
                .when_some(s.documentation.clone(), |d, doc| {
                    let clean = doc.replace("```python", "").replace("```", "");
                    d.child(div().mt(px(6.0)).text_color(t.fg_muted).children(clean.trim().lines().take(14).map(|l| div().child(l.to_owned()))))
                })
        });
        Some(
            deferred(
                anchored().position(point(anchor.left() - px(24.0), anchor.bottom() + px(2.0))).snap_to_window().child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(2.0))
                        .font_family(CODE_FONT)
                        .text_size(px(13.0))
                        .text_color(t.fg)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .id("completion-list")
                                .w(px(300.0))
                                .max_h(px(240.0))
                                .overflow_y_scroll()
                                .track_scroll(&self.completion.scroll)
                                .bg(t.bg_panel)
                                .border_1()
                                .border_color(t.line_strong)
                                .shadow_lg()
                                .children(rows),
                        )
                        .children(details),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    // ---------- hover ----------

    pub(crate) fn show_hover(&mut self, group: usize, pos: Pos, at: Point<Pixels>, cx: &mut Context<Self>) {
        let ed = self.editors[group].read(cx);
        let problems: Vec<(u8, String)> = ed
            .diagnostics
            .iter()
            .filter(|d| (d.start <= pos && pos <= d.end) || (d.start.line == pos.line && d.start == d.end && d.start.col == pos.col))
            .map(|d| (d.severity, d.message.clone()))
            .collect();
        let Some(doc) = ed.document().cloned() else { return };
        let path = doc.read(cx).doc.path().to_owned();
        let lsp_pos = self.to_lsp(&doc.read(cx).doc, pos);
        self.hover.generation += 1;
        let generation = self.hover.generation;
        self.hover.at = Some(at);
        self.hover.problems = problems;
        self.hover.sections.clear();
        cx.notify();
        let Some(server) = self.lsp.server.clone() else { return };
        self.lsp_flush(cx);
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            let md = server.hover(&path, lsp_pos).await;
            this.update(cx, |ws, cx| {
                if ws.hover.generation == generation {
                    ws.hover.sections = md.map(|m| hover_sections(&m)).unwrap_or_default();
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub(crate) fn hide_hover(&mut self, cx: &mut Context<Self>) {
        if self.hover.at.is_some() {
            self.hover.at = None;
            self.hover.generation += 1;
            cx.notify();
        }
    }

    pub(crate) fn render_hover(&self, t: &Theme) -> Option<AnyElement> {
        let at = self.hover.at?;
        if self.hover.sections.is_empty() && self.hover.problems.is_empty() {
            return None;
        }
        let problems = self.hover.problems.iter().map(|(sev, m)| {
            let color = match sev {
                1 => t.err,
                2 => t.warn,
                _ => t.info,
            };
            div().text_color(color).child(m.clone())
        });
        let sections = self.hover.sections.iter().map(|s| {
            if s.code {
                div().font_family(CODE_FONT).text_color(t.fg).children(s.text.lines().map(|l| div().child(l.to_owned())))
            } else {
                div().text_color(t.fg_muted).children(s.text.lines().take(20).map(|l| div().min_h(px(8.0)).child(l.to_owned())))
            }
        });
        Some(
            deferred(
                anchored().position(point(at.x, at.y + px(18.0))).snap_to_window().child(
                    div()
                        .max_w(px(560.0))
                        .p(px(8.0))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .bg(t.bg_panel)
                        .border_1()
                        .border_color(t.line_strong)
                        .shadow_lg()
                        .text_size(px(12.0))
                        .children(problems)
                        .children(sections),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    // ---------- go to definition, go back ----------

    /// Remember where we are before jumping (for Go Back).
    pub(crate) fn push_nav(&mut self, cx: &gpui_kit::App) {
        let ed = self.editors[self.tabs.active_group.min(1)].read(cx);
        if let Some(doc) = ed.document() {
            self.nav_back.push((PathBuf::from(doc.read(cx).doc.path()), ed.cursor()));
            if self.nav_back.len() > 100 {
                self.nav_back.remove(0);
            }
        }
    }

    pub(crate) fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, pos)) = self.nav_back.pop() else { return };
        self.open_path(path, window, cx);
        let ed = self.editors[self.tabs.active_group.min(1)].clone();
        ed.update(cx, |e, cx| e.go_to(pos.line, pos.col, cx));
    }

    /// Jump to where the name at `pos` (in the active editor) is defined.
    pub(crate) fn go_to_definition(&mut self, pos: Option<Pos>, window: &mut Window, cx: &mut Context<Self>) {
        let group = self.tabs.active_group.min(1);
        let ed = self.editors[group].clone();
        let Some(doc) = ed.read(cx).document().cloned() else { return };
        let pos = pos.unwrap_or_else(|| ed.read(cx).cursor());
        let path = doc.read(cx).doc.path().to_owned();
        let Some(server) = self.lsp.server.clone() else {
            return self.status_note("Go to Definition needs the Python language server.", cx);
        };
        self.lsp_flush(cx);
        let at = self.to_lsp(&doc.read(cx).doc, pos);
        cx.spawn_in(window, async move |this, cx| {
            let found = server.definition(&path, at).await;
            this.update_in(cx, |ws, window, cx| match found {
                Some(loc) => {
                    ws.push_nav(cx);
                    ws.open_at_lsp(PathBuf::from(&loc.path), loc.start.line, loc.start.col, window, cx);
                }
                None => ws.status_note("No definition found here.", cx),
            })
            .ok();
        })
        .detach();
    }

    /// The word at the cursor (for messages and prompts).
    pub(crate) fn word_at_cursor(&self, cx: &gpui_kit::App) -> Option<(Pos, Pos, String)> {
        let ed = self.editors[self.tabs.active_group.min(1)].read(cx);
        let doc = ed.document()?;
        let c = ed.cursor();
        let line: Vec<char> = doc.read(cx).doc.buffer.line(c.line).chars().collect();
        let mut s = c.col.min(line.len());
        while s > 0 && is_word_char(line[s - 1]) {
            s -= 1;
        }
        let mut e = c.col.min(line.len());
        while e < line.len() && is_word_char(line[e]) {
            e += 1;
        }
        (e > s).then(|| (Pos::new(c.line, s), Pos::new(c.line, e), line[s..e].iter().collect()))
    }
}
