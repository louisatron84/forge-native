//! The Problems panel: every error and warning from the language server (and
//! Ruff), grouped by file. Clicking one jumps to it. Replaces
//! `src/renderer/problemsView.ts`.

use crate::theme::ForgeTheme;
use forge_lsp::server::Diagnostic;
use gpui_kit::{
    div, prelude::FluentBuilder, AnyElement, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use std::path::PathBuf;

pub enum ProblemsEvent {
    /// Open a file at a server position (line, UTF-16 column).
    Open(PathBuf, usize, usize),
}

impl EventEmitter<ProblemsEvent> for ProblemsPanel {}

#[derive(Default)]
pub struct ProblemsPanel {
    /// (path, shown name, problems) per file.
    files: Vec<(String, String, Vec<Diagnostic>)>,
    /// Shown when there's nothing to list (e.g. "starting the language server…").
    pub status: String,
}

impl ProblemsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show these; returns (errors, warnings).
    pub fn set(
        &mut self,
        files: Vec<(String, Vec<Diagnostic>)>,
        relative: impl Fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> (usize, usize) {
        let mut shown: Vec<(String, String, Vec<Diagnostic>)> = files
            .into_iter()
            .map(|(p, l)| {
                let mut list: Vec<Diagnostic> = l.into_iter().filter(|d| d.severity <= 3).collect();
                list.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.start.cmp(&b.start)));
                let rel = relative(&p);
                (p, rel, list)
            })
            .filter(|(_, _, l)| !l.is_empty())
            .collect();
        shown.sort_by(|a, b| a.1.cmp(&b.1));
        let errors = shown.iter().flat_map(|f| &f.2).filter(|d| d.severity == 1).count();
        let warnings = shown.iter().flat_map(|f| &f.2).filter(|d| d.severity == 2).count();
        self.files = shown;
        cx.notify();
        (errors, warnings)
    }
}

impl Render for ProblemsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (fi, (path, rel, list)) in self.files.iter().enumerate() {
            rows.push(
                div()
                    .flex()
                    .gap(t.px(6.0))
                    .px(t.px(8.0))
                    .py(t.px(3.0))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(rel.clone())
                    .child(div().text_size(t.px(11.0)).text_color(t.fg_muted).child(list.len().to_string()))
                    .into_any_element(),
            );
            for (di, d) in list.iter().enumerate() {
                let (icon, color) = match d.severity {
                    1 => ("✖", t.err),
                    2 => ("⚠", t.warn),
                    _ => ("ℹ", t.info),
                };
                let where_ = format!(
                    "{}[{}, {}]",
                    d.code.as_ref().map(|c| format!("{c}  ")).unwrap_or_default(),
                    d.start.line + 1,
                    d.start.col + 1
                );
                let (p, line, col) = (PathBuf::from(path), d.start.line, d.start.col);
                rows.push(
                    div()
                        .id(SharedString::from(format!("problem-{fi}-{di}")))
                        .flex()
                        .items_center()
                        .gap(t.px(8.0))
                        .pl(t.px(20.0))
                        .pr(t.px(8.0))
                        .py(t.px(2.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.bg_hover))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(ProblemsEvent::Open(p.clone(), line, col))))
                        .child(div().w(t.px(14.0)).text_color(color).child(icon))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(d.message.lines().next().unwrap_or("").to_owned()),
                        )
                        .child(div().flex_none().text_size(t.px(11.0)).text_color(t.fg_muted).child(where_))
                        .into_any_element(),
                );
            }
        }
        let empty = rows.is_empty();
        let status = if self.status.is_empty() {
            "No problems found in open Python files.".to_owned()
        } else {
            self.status.clone()
        };
        div()
            .id("problems")
            .size_full()
            .overflow_y_scroll()
            .text_size(t.px(13.0))
            .text_color(t.fg)
            .py(t.px(4.0))
            .when(empty, |d| d.child(div().p(t.px(10.0)).text_color(t.fg_muted).child(status)))
            .children(rows)
    }
}
