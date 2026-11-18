//! Read-only Git diffs, hunk actions, history graph and blame.
//! Replaces `diffView.ts`, `historyView.ts` and the blame UI.

use crate::theme::{ForgeTheme, CODE_FONT};
use forge_core::git::{hunk_patch, parse_diff, LineKind};
use forge_core::history::{graph_layout, Commit, GraphRow};
use gpui_kit::{
    canvas, div, point, uniform_list, AnyElement, Context, EventEmitter, InteractiveElement, IntoElement,
    ParentElement, PathBuilder, Render, StatefulInteractiveElement, Styled, Window,
};

#[derive(Clone)]
pub enum InspectEvent {
    ApplyReviewed,
    Close,
    Patch { patch: String, cached: bool, reverse: bool },
    Commit(String),
    OpenLine(usize),
}

#[derive(Clone)]
struct Row {
    graph: Option<GraphRow>,
    text: String,
    kind: Option<LineKind>,
    event: Option<InspectEvent>,
    discard: Option<InspectEvent>,
}

#[derive(Default)]
pub struct GitInspect {
    pub action: Option<&'static str>,
    reject_label: Option<&'static str>,
    graph_columns: usize,
    pub open: bool,
    pub title: String,
    rows: Vec<Row>,
}

impl EventEmitter<InspectEvent> for GitInspect {}

impl GitInspect {
    #[cfg(test)]
    pub(crate) fn contains_text(&self, text: &str) -> bool {
        self.rows.iter().any(|row| row.text.contains(text))
    }
    pub fn text(&mut self, title: String, lines: Vec<String>, cx: &mut Context<Self>) {
        self.action = None;
        self.reject_label = None;
        self.title = title;
        self.open = true;
        self.graph_columns = 0;
        self.rows =
            lines.into_iter().map(|text| Row { graph: None, text, kind: None, event: None, discard: None }).collect();
        cx.notify();
    }

    pub fn review(
        &mut self,
        title: String,
        before: &str,
        after: &str,
        apply: &'static str,
        reject: &'static str,
        cx: &mut Context<Self>,
    ) {
        use similar::{Algorithm, ChangeTag, TextDiff};

        self.text(title, Vec::new(), cx);
        self.action = Some(apply);
        self.reject_label = Some(reject);
        let diff = TextDiff::configure()
            .algorithm(Algorithm::Patience)
            .timeout(std::time::Duration::from_millis(40))
            .diff_lines(before, after);
        for group in diff.grouped_ops(3) {
            if !self.rows.is_empty() {
                self.rows.push(Row { graph: None, text: "…".into(), kind: None, event: None, discard: None });
            }
            for operation in group {
                self.rows.extend(diff.iter_changes(&operation).map(|change| {
                    let kind = match change.tag() {
                        ChangeTag::Equal => LineKind::Context,
                        ChangeTag::Delete => LineKind::Removed,
                        ChangeTag::Insert => LineKind::Added,
                    };
                    let line = change.value().trim_end_matches(['\r', '\n']);
                    let end = if change.missing_newline() { "  (no newline)" } else { "" };
                    Row {
                        graph: None,
                        text: format!("{}{line}{end}", kind.prefix()),
                        kind: Some(kind),
                        event: None,
                        discard: None,
                    }
                }));
            }
        }
        if self.rows.is_empty() {
            self.rows.push(Row {
                graph: None,
                text: "No text changes.".into(),
                kind: None,
                event: None,
                discard: None,
            });
        }
    }

    pub fn diff(&mut self, title: String, text: &str, staged: bool, historical: bool, cx: &mut Context<Self>) {
        self.text(title, Vec::new(), cx);
        for file in parse_diff(text) {
            self.rows.push(Row { graph: None, text: file.head.join("  "), kind: None, event: None, discard: None });
            if file.binary {
                self.rows.push(Row {
                    graph: None,
                    text: "Binary files differ".into(),
                    kind: None,
                    event: None,
                    discard: None,
                });
            }
            for hunk in &file.hunks {
                let patch = hunk_patch(&file, hunk);
                self.rows.push(Row {
                    graph: None,
                    text: format!(
                        "{}{}",
                        hunk.header,
                        if historical {
                            ""
                        } else if staged {
                            "    [Unstage hunk]"
                        } else {
                            "    [Stage hunk]"
                        }
                    ),
                    kind: None,
                    event: (!historical).then(|| InspectEvent::Patch {
                        patch: patch.clone(),
                        cached: true,
                        reverse: staged,
                    }),
                    discard: (!historical && !staged).then_some(InspectEvent::Patch {
                        patch,
                        cached: false,
                        reverse: true,
                    }),
                });
                for line in &hunk.lines {
                    self.rows.push(Row {
                        graph: None,
                        text: format!(
                            "{:>5} {:>5} {}{}",
                            line.old_no.map(|n| n.to_string()).unwrap_or_default(),
                            line.new_no.map(|n| n.to_string()).unwrap_or_default(),
                            line.kind.prefix(),
                            line.text
                        ),
                        kind: Some(line.kind),
                        event: line.new_no.map(|n| InspectEvent::OpenLine(n.saturating_sub(1))),
                        discard: None,
                    });
                }
            }
        }
        if self.rows.is_empty() {
            self.text(self.title.clone(), vec!["No changes.".into()], cx);
        }
    }

    pub fn history(&mut self, title: String, commits: &[Commit], cx: &mut Context<Self>) {
        self.text(title, Vec::new(), cx);
        for (commit, graph) in commits.iter().zip(graph_layout(commits)) {
            let width = graph.before.len().max(graph.after.len()).max(graph.col + 1);
            self.graph_columns = self.graph_columns.max(width);
            self.rows.push(Row {
                graph: Some(graph),
                text: format!(
                    "{}  {}  {}  {}",
                    &commit.sha[..commit.sha.len().min(8)],
                    commit.subject,
                    commit.author,
                    commit.refs.join(", ")
                ),
                kind: None,
                event: Some(InspectEvent::Commit(commit.sha.clone())),
                discard: None,
            });
        }
        if commits.is_empty() {
            self.text(self.title.clone(), vec!["No commits yet.".into()], cx);
        }
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let row = &self.rows[index];
        let event = row.event.clone();
        let discard = row.discard.clone();
        let color = match row.kind {
            Some(LineKind::Added) => t.git_added,
            Some(LineKind::Removed) => t.git_deleted,
            _ => t.fg,
        };
        let mut el = div()
            .id(("git-inspect-row", index))
            .h(t.px(23.0))
            .flex()
            .items_center()
            .gap(t.px(12.0))
            .px(t.px(10.0))
            .text_color(color);
        if let Some(graph) = row.graph.clone() {
            let paint_theme = t.clone();
            let colors = [t.accent, t.git_added, t.git_modified, t.warn];
            el = el.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let t = &paint_theme;
                        let p = |column: usize, y: f32| {
                            point(bounds.origin.x + t.px(column as f32 * 14.0 + 7.0), bounds.origin.y + t.px(y))
                        };
                        let mut line = |from, to, color| {
                            let mut builder = PathBuilder::stroke(t.px(1.5));
                            builder.move_to(from);
                            builder.line_to(to);
                            if let Ok(path) = builder.build() {
                                window.paint_path(path, color);
                            }
                        };
                        for (column, target) in graph.before.iter().enumerate() {
                            if let Some(target) = target {
                                if column == graph.col || graph.joins.contains(&column) {
                                    line(p(column, 0.0), p(graph.col, 11.5), colors[column % 4]);
                                } else if let Some(next) =
                                    graph.after.iter().position(|value| value.as_ref() == Some(target))
                                {
                                    line(p(column, 0.0), p(next, 23.0), colors[column % 4]);
                                }
                            }
                        }
                        for column in &graph.parent_cols {
                            line(p(graph.col, 11.5), p(*column, 23.0), colors[*column % 4]);
                        }
                        let center = p(graph.col, 11.5);
                        let mut dot = PathBuilder::fill();
                        for i in 0..12 {
                            let angle = i as f32 * std::f32::consts::TAU / 12.0;
                            let position =
                                point(center.x + t.px(angle.cos() * 3.5), center.y + t.px(angle.sin() * 3.5));
                            if i == 0 {
                                dot.move_to(position);
                            } else {
                                dot.line_to(position);
                            }
                        }
                        dot.close();
                        if let Ok(path) = dot.build() {
                            window.paint_path(path, colors[graph.col % 4]);
                        }
                    },
                )
                .w(t.px(self.graph_columns as f32 * 14.0 + 8.0))
                .h_full()
                .flex_none(),
            );
        }
        el = el.child(div().flex_1().whitespace_nowrap().child(row.text.clone()));
        if let Some(event) = event {
            el = el
                .cursor_pointer()
                .hover(|s| s.bg(t.bg_hover))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone())));
        }
        if let Some(event) = discard {
            el = el.child(
                div()
                    .id(("discard-hunk", index))
                    .cursor_pointer()
                    .text_color(t.git_deleted)
                    .child("Discard hunk")
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(event.clone());
                    })),
            );
        }
        el.into_any_element()
    }
}

impl Render for GitInspect {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.fg)
            .child(
                div()
                    .flex()
                    .gap(t.px(12.0))
                    .px(t.px(10.0))
                    .py(t.px(6.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(self.title.clone())
                    .children(self.action.map(|label| {
                        div()
                            .id("inspect-action")
                            .debug_selector(|| "apply-generated-code".into())
                            .cursor_pointer()
                            .text_color(t.heat_2)
                            .child(label)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(InspectEvent::ApplyReviewed)))
                    }))
                    .child(
                        div()
                            .id("close-git-inspect")
                            .debug_selector(|| "reject-generated-change".into())
                            .ml_auto()
                            .cursor_pointer()
                            .child(self.reject_label.unwrap_or("Close"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(InspectEvent::Close))),
                    ),
            )
            .child(
                uniform_list(
                    "git-inspect",
                    self.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| range.map(|i| this.row(i, cx)).collect()),
                )
                .flex_1()
                .min_h_0()
                .font_family(CODE_FONT)
                .text_size(t.px(12.0)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn staged_and_historical_diffs_offer_only_valid_hunk_actions(cx: &mut TestAppContext) {
        let inspect = cx.new(|_| GitInspect::default());
        let patch = "diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -1 +1 @@\n-old\n+new\n";
        inspect.update(cx, |p, cx| {
            p.diff("Staged".into(), patch, true, false, cx);
            assert!(p
                .rows
                .iter()
                .any(|r| matches!(r.event, Some(InspectEvent::Patch { cached: true, reverse: true, .. }))));
            assert!(p.rows.iter().all(|r| r.discard.is_none()));
            p.diff("History".into(), patch, false, true, cx);
            assert!(p.rows.iter().all(|r| !matches!(r.event, Some(InspectEvent::Patch { .. })) && r.discard.is_none()));
            assert!(p.rows.iter().any(|r| r.kind == Some(LineKind::Added) && r.text.ends_with("+new")));
        });
    }
}
