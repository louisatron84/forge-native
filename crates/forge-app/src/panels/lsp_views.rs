//! The language server's lists: References (Shift+F12, and implementations),
//! Outline (the file's classes and functions) and Calls (who calls a function,
//! or what it calls, as a tree). Replaces `src/renderer/referencesView.ts`,
//! `outlineView.ts` and `callHierarchyView.ts`.

use crate::theme::{ForgeTheme, Theme};
use forge_lsp::navigation::{symbol_kind, CallItem, DocSymbol, Location, LspPos};
use forge_lsp::server::LspServer;
use gpui_kit::{
    div, prelude::FluentBuilder, AnyElement, Context, EventEmitter, FontWeight, HighlightStyle, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, StyledText, Task, Window,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Open a file at a server position (line, UTF-16 column).
pub struct OpenAt(pub PathBuf, pub LspPos);

fn empty(t: &Theme, text: &str) -> AnyElement {
    div().p(t.px(10.0)).text_color(t.fg_muted).child(text.to_owned()).into_any_element()
}

// ---------- References ----------

/// One found location, with the line's text (trimmed) and the found part in it (chars).
#[derive(Clone)]
pub struct Found {
    pub location: Location,
    pub preview: String,
    pub hl: (usize, usize),
}

#[derive(Default)]
pub struct ReferencesPanel {
    message: Option<String>,
    summary: String,
    /// (shown path, results) per file.
    groups: Vec<(String, Vec<Found>)>,
}

impl EventEmitter<OpenAt> for ReferencesPanel {}

impl ReferencesPanel {
    pub fn new() -> Self {
        Self {
            message: Some("Put the cursor on a name and press Shift+F12 to list everywhere it is used.".into()),
            ..Default::default()
        }
    }

    pub fn message(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some(text.into());
        self.groups.clear();
        cx.notify();
    }

    /// Show results; `what` names them ("reference", "subclass", "override").
    pub fn show(&mut self, name: &str, found: Vec<(String, Found)>, what: &str, cx: &mut Context<Self>) {
        let plural = if what.ends_with('s') { format!("{what}es") } else { format!("{what}s") };
        let to = if what == "reference" { "to" } else { "of" };
        if found.is_empty() {
            return self.message(format!("No {plural} {to} \"{name}\" found."), cx);
        }
        let mut groups: Vec<(String, Vec<Found>)> = Vec::new();
        for (rel, f) in found {
            match groups.iter_mut().find(|g| g.0 == rel) {
                Some(g) => g.1.push(f),
                None => groups.push((rel, vec![f])),
            }
        }
        let n: usize = groups.iter().map(|g| g.1.len()).sum();
        self.summary = format!(
            "{n} {} {to} \"{name}\" in {} file{}",
            if n == 1 { what.to_owned() } else { plural },
            groups.len(),
            if groups.len() == 1 { "" } else { "s" }
        );
        self.groups = groups;
        self.message = None;
        cx.notify();
    }
}

impl Render for ReferencesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(m) = &self.message {
            rows.push(empty(&t, m));
        } else {
            rows.push(
                div()
                    .px(t.px(10.0))
                    .py(t.px(4.0))
                    .text_color(t.fg_muted)
                    .text_size(t.px(12.0))
                    .child(self.summary.clone())
                    .into_any_element(),
            );
            for (gi, (rel, list)) in self.groups.iter().enumerate() {
                rows.push(
                    div()
                        .flex()
                        .gap(t.px(6.0))
                        .px(t.px(8.0))
                        .py(t.px(3.0))
                        .font_weight(FontWeight::MEDIUM)
                        .child(rel.clone())
                        .child(div().text_size(t.px(11.0)).text_color(t.fg_muted).child(list.len().to_string()))
                        .into_any_element(),
                );
                for (fi, f) in list.iter().enumerate() {
                    let start = f.preview.char_indices().nth(f.hl.0).map_or(f.preview.len(), |(b, _)| b);
                    let end = f.preview.char_indices().nth(f.hl.1).map_or(f.preview.len(), |(b, _)| b);
                    let styled = StyledText::new(f.preview.clone()).with_highlights([(
                        start..end.max(start),
                        HighlightStyle { background_color: Some(t.match_bg), color: Some(t.fg), ..Default::default() },
                    )]);
                    let (path, at) = (PathBuf::from(&f.location.path), f.location.start);
                    rows.push(
                        div()
                            .id(SharedString::from(format!("ref-{gi}-{fi}")))
                            .flex()
                            .gap(t.px(8.0))
                            .pl(t.px(20.0))
                            .pr(t.px(8.0))
                            .py(t.px(1.0))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.bg_hover))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenAt(path.clone(), at))))
                            .child(
                                div()
                                    .min_w(t.px(28.0))
                                    .text_size(t.px(11.0))
                                    .text_color(t.fg_muted)
                                    .child((f.location.start.line + 1).to_string()),
                            )
                            .child(div().text_color(t.fg_muted).child(styled))
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .id("references")
            .size_full()
            .overflow_y_scroll()
            .text_size(t.px(13.0))
            .text_color(t.fg)
            .py(t.px(4.0))
            .children(rows)
    }
}

// ---------- Outline ----------

#[derive(Default)]
pub struct OutlinePanel {
    symbols: Vec<DocSymbol>,
    message: Option<String>,
    /// Names from the outermost symbol to the one the cursor is in.
    current: Vec<(LspPos, String)>,
}

impl EventEmitter<OpenAt> for OutlinePanel {}

impl OutlinePanel {
    pub fn new() -> Self {
        Self { message: Some("Open a Python file to see its outline.".into()), ..Default::default() }
    }

    pub fn set(&mut self, symbols: Vec<DocSymbol>, message: Option<String>, cx: &mut Context<Self>) {
        self.symbols = symbols;
        self.message = message;
        cx.notify();
    }

    /// Highlight the symbols around the cursor.
    pub fn highlight(&mut self, at: Option<LspPos>, cx: &mut Context<Self>) {
        let path = at.map(|p| forge_lsp::navigation::symbol_path_at(&self.symbols, p)).unwrap_or_default();
        let current: Vec<(LspPos, String)> = path.into_iter().map(|s| (s.name_start, s.name)).collect();
        if current != self.current {
            self.current = current;
            cx.notify();
        }
    }

    fn rows(&self, list: &[DocSymbol], depth: usize, t: &Theme, out: &mut Vec<AnyElement>, cx: &mut Context<Self>) {
        for s in list {
            let (icon, _) = symbol_kind(s.kind);
            let on = self.current.last().is_some_and(|c| c.0 == s.name_start && c.1 == s.name);
            let at = s.name_start;
            out.push(
                div()
                    .id(SharedString::from(format!("outline-{}-{}-{}", s.name_start.line, s.name_start.col, out.len())))
                    .flex()
                    .items_center()
                    .gap(t.px(6.0))
                    .pl(t.px(8.0 + depth as f32 * 14.0))
                    .pr(t.px(8.0))
                    .h(t.px(22.0))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when(on, |d| d.bg(t.bg_active))
                    .when(!on, |d| d.hover(|x| x.bg(t.bg_hover)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenAt(PathBuf::new(), at))))
                    .child(div().w(t.px(14.0)).text_color(t.accent).child(icon))
                    .child(s.name.clone())
                    .when_some(s.detail.clone(), |d, detail| {
                        d.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(t.px(11.0))
                                .text_color(t.fg_muted)
                                .child(detail),
                        )
                    })
                    .into_any_element(),
            );
            self.rows(&s.children, depth + 1, t, out, cx);
        }
    }
}

impl Render for OutlinePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut rows = Vec::new();
        match &self.message {
            Some(m) => rows.push(empty(&t, m)),
            None if self.symbols.is_empty() => rows.push(empty(&t, "No classes or functions in this file.")),
            None => {
                let symbols = self.symbols.clone();
                self.rows(&symbols, 0, &t, &mut rows, cx);
            }
        }
        div()
            .id("outline")
            .size_full()
            .overflow_y_scroll()
            .text_size(t.px(13.0))
            .text_color(t.fg)
            .py(t.px(4.0))
            .children(rows)
    }
}

// ---------- Calls ----------

/// Calls nested deeper than this are not expanded (recursion can go on forever).
const MAX_DEPTH: usize = 12;

struct CallNode {
    item: CallItem,
    /// Where the calls are (in this item's file for incoming, the parent's for outgoing).
    at: Vec<LspPos>,
    parent_path: Option<String>,
    expanded: bool,
    loading: bool,
    children: Option<Vec<CallNode>>,
    recursive: bool,
}

#[derive(Default)]
pub struct CallsPanel {
    server: Option<Arc<LspServer>>,
    root: Option<CallNode>,
    incoming: bool,
    message: Option<String>,
    generation: u64,
    tasks: Vec<Task<()>>,
}

impl EventEmitter<OpenAt> for CallsPanel {}

fn key(i: &CallItem) -> String {
    format!("{}:{}:{}", i.path, i.name_start.line, i.name_start.col)
}

impl CallsPanel {
    pub fn new() -> Self {
        Self {
            incoming: true,
            message: Some("Put the cursor on a function and press Shift+Alt+H to see who calls it.".into()),
            ..Default::default()
        }
    }

    pub fn message(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.generation += 1;
        self.root = None;
        self.message = Some(text.into());
        cx.notify();
    }

    pub fn show(&mut self, server: Arc<LspServer>, item: CallItem, cx: &mut Context<Self>) {
        self.server = Some(server);
        self.incoming = true;
        self.message = None;
        self.reset_root(item, cx);
    }

    fn reset_root(&mut self, item: CallItem, cx: &mut Context<Self>) {
        self.generation += 1;
        self.root = Some(CallNode {
            item,
            at: Vec::new(),
            parent_path: None,
            expanded: false,
            loading: false,
            children: None,
            recursive: false,
        });
        self.toggle(&[], cx);
    }

    fn node_mut<'a>(node: &'a mut CallNode, path: &[usize]) -> Option<&'a mut CallNode> {
        match path.split_first() {
            None => Some(node),
            Some((i, rest)) => Self::node_mut(node.children.as_mut()?.get_mut(*i)?, rest),
        }
    }

    /// Expand or fold a node (its calls load the first time).
    fn toggle(&mut self, path: &[usize], cx: &mut Context<Self>) {
        let incoming = self.incoming;
        let generation = self.generation;
        let Some(root) = self.root.as_mut() else { return };
        // The keys of the node's ancestors (and itself), to spot recursion.
        let mut seen = Vec::new();
        let mut n: &CallNode = root;
        seen.push(key(&n.item));
        for i in path {
            let Some(c) = n.children.as_ref().and_then(|c| c.get(*i)) else { return };
            n = c;
            seen.push(key(&n.item));
        }
        let Some(node) = Self::node_mut(root, path) else { return };
        if node.recursive || path.len() >= MAX_DEPTH {
            return;
        }
        node.expanded = !node.expanded;
        if !node.expanded || node.children.is_some() || node.loading {
            cx.notify();
            return;
        }
        node.loading = true;
        let item = node.item.clone();
        let Some(server) = self.server.clone() else { return };
        let path = path.to_vec();
        let parent = item.path.clone();
        self.tasks.push(cx.spawn(async move |this, cx| {
            let calls = server.calls(&item, incoming).await;
            this.update(cx, |p, cx| {
                if p.generation != generation {
                    return;
                }
                let Some(root) = p.root.as_mut() else { return };
                if let Some(node) = Self::node_mut(root, &path) {
                    node.loading = false;
                    node.children = Some(
                        calls
                            .into_iter()
                            .map(|(item, at)| {
                                let recursive = seen.contains(&key(&item));
                                CallNode {
                                    item,
                                    at,
                                    parent_path: Some(parent.clone()),
                                    expanded: false,
                                    loading: false,
                                    children: None,
                                    recursive,
                                }
                            })
                            .collect(),
                    );
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn rows(&self, node: &CallNode, path: Vec<usize>, t: &Theme, out: &mut Vec<AnyElement>, cx: &mut Context<Self>) {
        let depth = path.len();
        let (icon, _) = symbol_kind(node.item.kind);
        let twisty = if node.recursive || depth >= MAX_DEPTH {
            " "
        } else if node.expanded {
            "▾"
        } else {
            "▸"
        };
        let where_ = format!(
            "{}{}:{}",
            node.item.detail.as_ref().map(|d| format!("{d}  ")).unwrap_or_default(),
            node.item.path.rsplit(['/', '\\']).next().unwrap_or(""),
            node.item.name_start.line + 1
        );
        let (p1, p2) = (path.clone(), path.clone());
        let (item_path, item_at) = (PathBuf::from(&node.item.path), node.item.name_start);
        let site = node.at.first().copied().map(|a| {
            let file = if self.incoming {
                node.item.path.clone()
            } else {
                node.parent_path.clone().unwrap_or_else(|| node.item.path.clone())
            };
            (PathBuf::from(file), a, node.at.len())
        });
        out.push(
            div()
                .id(SharedString::from(format!("calls-{path:?}")))
                .flex()
                .items_center()
                .gap(t.px(6.0))
                .pl(t.px(8.0 + depth as f32 * 16.0))
                .pr(t.px(8.0))
                .h(t.px(22.0))
                .whitespace_nowrap()
                .cursor_pointer()
                .hover(|s| s.bg(t.bg_hover))
                .on_click(cx.listener(move |_, e: &gpui_kit::ClickEvent, _, cx| {
                    if e.click_count() >= 2 {
                        return;
                    }
                    cx.emit(OpenAt(item_path.clone(), item_at))
                }))
                .child(
                    div()
                        .id(SharedString::from(format!("calls-twisty-{p1:?}")))
                        .w(t.px(12.0))
                        .text_color(t.fg_muted)
                        .on_click(cx.listener(move |p, _, _, cx| {
                            cx.stop_propagation();
                            p.toggle(&p2, cx);
                        }))
                        .child(twisty),
                )
                .child(div().w(t.px(14.0)).text_color(t.accent).child(icon))
                .child(format!("{}{}", node.item.name, if node.recursive { " (recursive)" } else { "" }))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(t.px(11.0))
                        .text_color(t.fg_muted)
                        .child(where_),
                )
                .when_some(site, |d, (file, at, n)| {
                    d.child(
                        div()
                            .id(SharedString::from(format!("calls-site-{path:?}")))
                            .ml_auto()
                            .px(t.px(6.0))
                            .text_size(t.px(11.0))
                            .text_color(t.link)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(OpenAt(file.clone(), at));
                            }))
                            .child(format!("{n} call{}", if n == 1 { "" } else { "s" })),
                    )
                })
                .into_any_element(),
        );
        if !node.expanded {
            return;
        }
        match &node.children {
            None => out.push(
                div()
                    .pl(t.px(24.0 + (depth + 1) as f32 * 16.0))
                    .text_color(t.fg_muted)
                    .child("Loading…")
                    .into_any_element(),
            ),
            Some(c) if c.is_empty() => out.push(
                div()
                    .pl(t.px(24.0 + (depth + 1) as f32 * 16.0))
                    .text_color(t.fg_muted)
                    .child(if self.incoming { "No callers found." } else { "No calls found." })
                    .into_any_element(),
            ),
            Some(children) => {
                for (i, c) in children.iter().enumerate() {
                    let mut p = path.clone();
                    p.push(i);
                    self.rows(c, p, t, out, cx);
                }
            }
        }
    }
}

impl Render for CallsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut rows = Vec::new();
        if let Some(m) = &self.message {
            rows.push(empty(&t, m));
        }
        let title = self.root.as_ref().map(|r| {
            if self.incoming {
                format!("Who calls {}", r.item.name)
            } else {
                format!("What {} calls", r.item.name)
            }
        });
        if let Some(root) = &self.root {
            let mut out = Vec::new();
            self.rows(root, Vec::new(), &t, &mut out, cx);
            rows.extend(out);
        }
        div()
            .id("calls")
            .size_full()
            .overflow_y_scroll()
            .text_size(t.px(13.0))
            .text_color(t.fg)
            .py(t.px(4.0))
            .when_some(title, |d, title| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(t.px(10.0))
                        .px(t.px(10.0))
                        .py(t.px(4.0))
                        .text_size(t.px(12.0))
                        .text_color(t.fg_muted)
                        .child(title)
                        .child(
                            div()
                                .id("calls-direction")
                                .px(t.px(6.0))
                                .border_1()
                                .border_color(t.border)
                                .cursor_pointer()
                                .hover(|s| s.bg(t.bg_hover))
                                .on_click(cx.listener(|p, _, _, cx| {
                                    p.incoming = !p.incoming;
                                    if let Some(item) = p.root.as_ref().map(|r| r.item.clone()) {
                                        p.reset_root(item, cx);
                                    }
                                }))
                                .child(if self.incoming { "⇄ Show what it calls" } else { "⇄ Show who calls it" }),
                        ),
                )
            })
            .children(rows)
    }
}
