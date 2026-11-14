use crate::theme::ForgeTheme;
use forge_core::{
    navigation::{file_summary, neighbourhood, project_map, Rect},
    project_graph::ProjectGraph,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, prelude::FluentBuilder, relative, uniform_list, AppContext, Context, Entity, EventEmitter, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone)]
pub enum NavigationEvent {
    OpenFile(String),
    OpenModule(String, usize),
    CloseMap,
}
pub struct ContextRow {
    pub label: String,
    pub detail: String,
    pub target: Option<(String, usize)>,
}
pub struct ContextPanel {
    pub rows: Vec<ContextRow>,
    graph: Arc<ProjectGraph>,
    module: Option<String>,
    infile: bool,
}
impl EventEmitter<NavigationEvent> for ContextPanel {}
impl ContextPanel {
    pub fn new(infile: bool) -> Self {
        Self { rows: Vec::new(), graph: Arc::default(), module: None, infile }
    }
    pub fn show(&mut self, graph: Arc<ProjectGraph>, module: Option<String>, cx: &mut Context<Self>) {
        if Arc::ptr_eq(&self.graph, &graph) && self.module == module {
            return;
        }
        self.rows.clear();
        if let Some(module) = &module {
            if self.infile {
                for s in file_summary(&graph, module) {
                    self.rows.push(ContextRow {
                        label: format!("{} {}", if s.kind == "class" { "C" } else { "ƒ" }, s.qual),
                        detail: s.summary,
                        target: Some((module.clone(), s.line.saturating_sub(1))),
                    });
                }
            } else {
                let n = neighbourhood(&graph, module);
                for (label, items) in [("Imports", n.imports), ("Imported by", n.imported_by)] {
                    for id in items {
                        self.rows.push(ContextRow { label: id.clone(), detail: label.into(), target: Some((id, 0)) });
                    }
                }
                for host in n.hosts {
                    self.rows.push(ContextRow { label: host, detail: "Web API".into(), target: None });
                }
            }
        }
        self.module = module;
        self.graph = graph;
        cx.notify();
    }
}
impl Render for ContextPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(t.bg_panel)
            .text_color(t.fg)
            .text_size(t.px(12.))
            .child(
                div()
                    .p(t.px(8.))
                    .text_color(t.heat_2)
                    .child(self.module.clone().unwrap_or_else(|| "Open a Python file to explore its context.".into())),
            )
            .child(
                uniform_list(
                    "file-context",
                    self.rows.len(),
                    cx.processor(|p, range: std::ops::Range<usize>, _, cx| {
                        let t = cx.global::<ForgeTheme>().0.clone();
                        range
                            .map(|i| {
                                let row = &p.rows[i];
                                let target = row.target.clone();
                                div()
                                    .id(("file-context-row", i))
                                    .h(t.px(46.))
                                    .px(t.px(8.))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_col()
                                    .child(row.label.clone())
                                    .child(
                                        div()
                                            .text_color(t.fg_muted)
                                            .text_size(t.px(10.))
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(row.detail.clone()),
                                    )
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if let Some((module, line)) = &target {
                                            cx.emit(NavigationEvent::OpenModule(module.clone(), *line));
                                        }
                                    }))
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
    }
}
pub struct MapPanel {
    pub files: Vec<(String, usize)>,
    pub current: Option<String>,
    pub problems: BTreeSet<String>,
    pub changed: BTreeSet<String>,
    pub recent: BTreeSet<String>,
    query: Entity<InputState>,
    _sub: Subscription,
}
impl EventEmitter<NavigationEvent> for MapPanel {}
impl MapPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find a file… Enter opens the first match"));
        let sub = cx.subscribe(&query, |p, _, event, cx| match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => {
                let q = p.query.read(cx).value().to_lowercase();
                if let Some((path, _)) =
                    p.files.iter().find(|(path, _)| !q.trim().is_empty() && path.to_lowercase().contains(q.trim()))
                {
                    cx.emit(NavigationEvent::OpenFile(path.clone()));
                }
            }
            _ => {}
        });
        Self {
            files: Vec::new(),
            current: None,
            problems: BTreeSet::new(),
            changed: BTreeSet::new(),
            recent: BTreeSet::new(),
            query,
            _sub: sub,
        }
    }
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |q, cx| {
            q.set_value("", window, cx);
            q.focus(window, cx);
        });
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.current = None;
        self.problems.clear();
        self.changed.clear();
        self.recent.clear();
        cx.notify();
    }
}
impl Render for MapPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let layout = project_map(&self.files, Rect { x: 0., y: 0., w: 1000., h: 600. });
        let q = self.query.read(cx).value().to_lowercase();
        let group_tiles = layout
            .groups
            .iter()
            .map(|(name, r)| {
                div()
                    .absolute()
                    .left(relative((r.x / 1000.) as f32))
                    .top(relative((r.y / 600.) as f32))
                    .w(relative((r.w / 1000.) as f32))
                    .h(relative((r.h / 600.) as f32))
                    .bg(t.bg_active)
                    .text_size(t.px(10.))
                    .text_color(t.fg_muted)
                    .overflow_hidden()
                    .child(if name.is_empty() { "(top level)".into() } else { format!("{name}/") })
            })
            .collect::<Vec<_>>();
        let files = self
            .files
            .iter()
            .filter_map(|(path, lines)| {
                let r = layout.files.get(path)?;
                let path = path.clone();
                let selector = format!("project-map-file-{path}");
                let border = if self.problems.contains(&path) {
                    t.err
                } else if self.current.as_ref() == Some(&path) {
                    t.heat_2
                } else if self.changed.contains(&path) {
                    t.warn
                } else if self.recent.contains(&path) {
                    t.graph_in
                } else {
                    t.border
                };
                Some(
                    div()
                        .id(SharedString::from(path.clone()))
                        .debug_selector(move || selector.clone())
                        .absolute()
                        .left(relative((r.x / 1000.) as f32))
                        .top(relative((r.y / 600.) as f32))
                        .w(relative((r.w / 1000.) as f32))
                        .h(relative((r.h / 600.) as f32))
                        .overflow_hidden()
                        .p(t.px(3.))
                        .border_1()
                        .border_color(border)
                        .bg(t.raised)
                        .when(!q.is_empty() && !path.to_lowercase().contains(q.trim()), |d| d.opacity(0.25))
                        .text_size(t.px(11.))
                        .cursor_pointer()
                        .child(path.rsplit('/').next().unwrap_or(&path).to_string())
                        .when(r.h > 32., |d| {
                            d.child(div().text_color(t.fg_muted).text_size(t.px(10.)).child(format!("{lines} lines")))
                        })
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(NavigationEvent::OpenFile(path.clone())))),
                )
            })
            .collect::<Vec<_>>();
        div().size_full().min_h_0().flex().flex_col().bg(t.bg_panel).text_color(t.fg).child(div().id("project-map-filter").p(t.px(8.)).child(Input::new(&self.query))).child(div().id("project-map-board").relative().flex_1().min_h_0().overflow_hidden().children(group_tiles).children(files)).child(div().p(t.px(7.)).text_size(t.px(10.)).text_color(t.fg_muted).child("Files are sized by lines and grouped by folder · orange: open · red: problems · yellow: changed · blue: recently opened")).on_key_down(cx.listener(|_,event:&gpui_kit::KeyDownEvent,_,cx|{if event.keystroke.key=="escape"{cx.emit(NavigationEvent::CloseMap);cx.stop_propagation();}}))
    }
}
