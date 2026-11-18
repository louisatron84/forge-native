//! Native function pipelines: virtual cards, dependency wires and captured values.
use crate::theme::ForgeTheme;
use forge_core::{
    data_summary::{change, summary_text, DataLog, DataMessage, Summary},
    dataflow::{dependencies, Direction, Flow, FlowNode, RawFlow},
};
use gpui_kit::{
    canvas, div, point, prelude::FluentBuilder, px, uniform_list, AnyElement, Context, EventEmitter,
    InteractiveElement, IntoElement, ParentElement, PathBuilder, Render, ScrollStrategy, SharedString,
    StatefulInteractiveElement, Styled, Task, UniformListScrollHandle, Window,
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};
#[derive(Clone)]
pub enum PipelineEvent {
    PickFunction,
    Refresh,
    Run,
    Open(u32),
    AddBlock(bool),
    New,
    Export,
    Argument(usize),
    EditBlock,
}
pub struct PipelinePanel {
    pub raw: Option<RawFlow>,
    pub flow: Option<Arc<Flow>>,
    pub func: String,
    pub module: String,
    pub rel: Option<String>,
    pub selected: Option<String>,
    pub upstream: BTreeSet<String>,
    pub downstream: BTreeSet<String>,
    pub log: DataLog,
    pub status: String,
    pub loading: bool,
    edges: Arc<Vec<(usize, usize)>>,
    refresh_task: Option<Task<()>>,
    capture_valid: bool,
    scroll: UniformListScrollHandle,
}
impl EventEmitter<PipelineEvent> for PipelinePanel {}
impl PipelinePanel {
    pub fn new() -> Self {
        Self {
            raw: None,
            flow: None,
            func: String::new(),
            module: String::new(),
            rel: None,
            selected: None,
            upstream: BTreeSet::new(),
            downstream: BTreeSet::new(),
            log: DataLog::default(),
            status: "Open a Python file to see its functions as pipelines.".into(),
            loading: false,
            edges: Arc::default(),
            refresh_task: None,
            capture_valid: false,
            scroll: UniformListScrollHandle::new(),
        }
    }
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        *self = Self::new();
        cx.notify();
    }
    pub fn invalidate_data(&mut self, cx: &mut Context<Self>) {
        self.log.begin(self.log.run_id);
        self.capture_valid = false;
        self.refresh_task = None;
        cx.notify();
    }
    pub fn begin(&mut self, id: u64, cx: &mut Context<Self>) {
        self.log.begin(id);
        self.capture_valid = true;
        self.refresh_task = None;
        cx.notify();
    }
    pub fn add(&mut self, m: DataMessage, cx: &mut Context<Self>) {
        if !self.capture_valid {
            return;
        }
        self.log.add(m);
        if self.refresh_task.is_none() {
            self.refresh_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(32)).await;
                this.update(cx, |p, cx| {
                    p.refresh_task = None;
                    cx.notify();
                })
                .ok();
            }));
        }
    }
    pub fn set_flow(&mut self, raw: RawFlow, flow: Option<Flow>, cx: &mut Context<Self>) {
        self.loading = false;
        self.status = raw.error.clone().unwrap_or_default();
        self.func = raw.func.clone().unwrap_or_default();
        let mut edges = Vec::new();
        if let Some(flow) = &flow {
            let index: HashMap<_, _> = flow.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
            edges = flow
                .edges
                .iter()
                .filter_map(|e| Some((*index.get(e.from.as_str())?, *index.get(e.to.as_str())?)))
                .collect();
        }
        self.edges = Arc::new(edges);
        self.raw = Some(raw);
        self.flow = flow.map(Arc::new);
        if self
            .selected
            .as_ref()
            .is_some_and(|id| self.flow.as_ref().is_none_or(|f| !f.nodes.iter().any(|n| &n.id == id)))
        {
            self.selected = None;
        }
        self.highlight();
        cx.notify();
    }
    fn highlight(&mut self) {
        self.upstream.clear();
        self.downstream.clear();
        if let Some((flow, id)) = self.flow.as_ref().zip(self.selected.as_ref()) {
            self.upstream = dependencies(flow, id, Direction::Up);
            self.downstream = dependencies(flow, id, Direction::Down);
        }
    }
    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(i) = self.flow.as_ref().and_then(|f| f.nodes.iter().position(|n| n.id == id)) {
            self.scroll.scroll_to_item(i, ScrollStrategy::Center);
        }
        self.selected = if self.selected.as_ref() == Some(&id) { None } else { Some(id) };
        self.highlight();
        cx.notify();
    }
    fn func_id(&self) -> String {
        format!("{}:{}", self.module, self.func)
    }
    fn value(&self, n: &FlowNode, name: &str) -> Option<Arc<Summary>> {
        let func = self.func_id();
        if n.kind == "param" {
            return self.log.io(&func)?.calls.first()?.input.get(name).cloned();
        }
        self.log
            .between(&func, n.line, if n.container() { n.line } else { n.end_line })
            .into_iter()
            .find(|p| p.var == name)
            .map(|p| p.summary.clone())
    }
    fn output_text(&self, n: &FlowNode) -> String {
        let names =
            if n.sets.is_empty() && n.kind == "return" { vec!["(returned)".to_string()] } else { n.sets.clone() };
        names
            .iter()
            .take(2)
            .map(|name| {
                self.value(n, name).map_or_else(|| format!("→ {name}"), |s| format!("→ {name}: {}", summary_text(&s)))
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
    fn transformation(&self, n: &FlowNode) -> String {
        let Some(flow) = &self.flow else { return String::new() };
        let first = n.call.as_ref().into_iter().flat_map(|c| &c.args).flat_map(|a| &a.names).find_map(|name| {
            let origin = flow.origins.get(&n.id)?.get(name)?;
            self.value(flow.nodes.iter().find(|n| &n.id == origin)?, name)
        });
        let output = n.sets.iter().find_map(|name| self.value(n, name));
        let kind = n.call.as_ref().map(|c| if c.name.contains("group") { "group" } else { "" }).unwrap_or_default();
        change(first.as_deref(), output.as_deref(), kind)
    }
    fn row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let flow = self.flow.as_ref().expect("rows only exist for flow");
        let n = &flow.nodes[i];
        let id = n.id.clone();
        let selector = format!("pipeline-node-{id}");
        let selected = self.selected.as_ref() == Some(&id);
        let up = self.upstream.contains(&id);
        let down = self.downstream.contains(&id);
        let color = if selected {
            t.heat_2
        } else if up {
            t.graph_in
        } else if down {
            t.graph_out
        } else {
            t.line_strong
        };
        let edges = self.edges.clone();
        let wires_theme = t.clone();
        let height = t.px(112.);
        let lit = self.selected.clone();
        let nodes = self.flow.clone();
        let wires = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                for (k, (from, to)) in edges.iter().enumerate() {
                    let (a, b) = ((*from).min(*to), (*from).max(*to));
                    if i < a || i > b {
                        continue;
                    }
                    let lane = bounds.left() + px(7. + (k % 4) as f32 * 7.);
                    let x = bounds.right();
                    let mid = bounds.top() + height * 0.5;
                    let start = if i == *from { point(x, mid) } else { point(lane, bounds.top()) };
                    let end = if i == *to { point(x, mid) } else { point(lane, bounds.bottom()) };
                    let mut path = PathBuilder::stroke(px(1.));
                    path.move_to(start);
                    if i == *from {
                        path.line_to(point(lane, mid));
                    }
                    if i == *to {
                        path.line_to(point(lane, mid));
                    }
                    path.line_to(end);
                    let on = nodes.as_ref().is_some_and(|f| {
                        lit.as_ref().is_some_and(|id| f.nodes[*from].id == *id || f.nodes[*to].id == *id)
                    });
                    if let Ok(path) = path.build() {
                        window.paint_path(path, if on { wires_theme.heat_2 } else { wires_theme.line_strong });
                    }
                    if i == *to {
                        let mut arrow = PathBuilder::stroke(px(1.));
                        arrow.move_to(point(x - px(4.), mid - px(3.)));
                        arrow.line_to(point(x, mid));
                        arrow.line_to(point(x - px(4.), mid + px(3.)));
                        if let Ok(path) = arrow.build() {
                            window.paint_path(path, if on { wires_theme.heat_2 } else { wires_theme.line_strong });
                        }
                    }
                }
            },
        )
        .w(t.px(40.))
        .h(height);
        div()
            .h(height)
            .w_full()
            .flex()
            .min_w_0()
            .child(wires)
            .child(
                div()
                    .id(SharedString::from(id.clone()))
                    .debug_selector(move || selector.clone())
                    .flex_1()
                    .min_w_0()
                    .my(t.px(4.))
                    .mr(t.px(6.))
                    .ml(t.px((n.depth.min(8) * 12) as f32))
                    .p(t.px(8.))
                    .border_1()
                    .border_color(color)
                    .rounded(t.corner(t.px(3.)))
                    .bg(if selected { t.bg_active } else { t.raised })
                    .when(self.selected.is_some() && !selected && !up && !down, |d| d.opacity(0.5))
                    .overflow_hidden()
                    .cursor_pointer()
                    .flex()
                    .flex_col()
                    .gap(t.px(4.))
                    .child(div().text_size(t.px(10.)).text_color(t.fg_muted).child(format!(
                        "{} · line {}",
                        if n.kind == "param" { "input" } else { &n.kind },
                        n.line
                    )))
                    .child(div().text_ellipsis().whitespace_nowrap().child(n.label.clone()))
                    .child(
                        div()
                            .text_size(t.px(10.))
                            .text_color(t.graph_out)
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(self.output_text(n)),
                    )
                    .child(
                        div()
                            .text_size(t.px(10.))
                            .text_color(t.heat_2)
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(self.transformation(n)),
                    )
                    .on_click(cx.listener(move |p, _, _, cx| p.select(id.clone(), cx))),
            )
            .into_any_element()
    }
    fn summary_view(&self, title: String, s: &Summary, cx: &Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut out = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(t.px(4.))
            .border_t_1()
            .border_color(t.border)
            .py(t.px(8.))
            .child(div().text_color(t.graph_out).child(title.clone()))
            .child(summary_text(s));
        if let Some(rows) = &s.rows {
            let cell = |value: String| {
                div().w(t.px(90.)).flex_none().px(t.px(4.)).text_ellipsis().whitespace_nowrap().child(value)
            };
            let columns = s.columns.as_ref().map_or(0, |c| c.len()).max(rows.first().map_or(0, Vec::len)).min(20);
            let mut table = div().w(t.px(32. + columns as f32 * 90.)).flex().flex_col().gap(t.px(4.));
            if let Some(cols) = &s.columns {
                table = table.child(
                    div().flex().text_color(t.fg_muted).child(div().w(t.px(32.)).flex_none().child("#")).children(
                        cols.iter().take(20).map(|(name, dtype)| {
                            cell(if dtype.is_empty() { name.clone() } else { format!("{name}: {dtype}") })
                        }),
                    ),
                );
            }
            for (i, row) in rows.iter().take(5).enumerate() {
                table =
                    table.child(
                        div()
                            .flex()
                            .child(div().w(t.px(32.)).flex_none().child(
                                s.index.as_ref().and_then(|v| v.get(i)).cloned().unwrap_or_else(|| i.to_string()),
                            ))
                            .children(row.iter().take(20).map(|v| cell(v.clone()))),
                    );
            }
            let selector = format!("pipeline-preview-{title}");
            out = out.child(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .w_full()
                    .min_w_0()
                    .flex_none()
                    .overflow_x_scroll()
                    .child(table),
            );
        }
        if let Some(samples) = &s.sample {
            for value in samples.iter().take(5) {
                out = out.child(value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string()));
            }
        }
        out.into_any_element()
    }
    fn details(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut out = div()
            .id("pipeline-details")
            .w(t.px(270.))
            .min_w(t.px(180.))
            .h_full()
            .overflow_y_scroll()
            .p(t.px(10.))
            .border_l_1()
            .border_color(t.border)
            .text_size(t.px(11.))
            .flex()
            .flex_col()
            .gap(t.px(8.));
        let Some(flow) = &self.flow else { return out.into_any_element() };
        let node = self.selected.as_ref().and_then(|id| flow.nodes.iter().find(|n| &n.id == id));
        if let Some(n) = node {
            out = out.child(n.label.clone());
            if let Some(call) = &n.call {
                for (i, arg) in call.args.iter().enumerate() {
                    out = out.child(
                        div()
                            .id(("pipeline-arg", i))
                            .cursor_pointer()
                            .text_color(t.link)
                            .child(format!(
                                "{}{}",
                                arg.kw.as_ref().map(|k| format!("{k} = ")).unwrap_or_default(),
                                arg.text
                            ))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(PipelineEvent::Argument(i)))),
                    );
                }
                out = out.child(
                    div()
                        .id("pipeline-edit-block")
                        .cursor_pointer()
                        .text_color(t.link)
                        .child("Edit block source…")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(PipelineEvent::EditBlock))),
                );
            }
            let line = n.line;
            out = out.child(
                div()
                    .id("pipeline-open-source")
                    .debug_selector(|| "pipeline-open-source".into())
                    .cursor_pointer()
                    .text_color(t.link)
                    .child("Open in editor")
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PipelineEvent::Open(line)))),
            );
            if let Some(origins) = flow.origins.get(&n.id) {
                out = out.child(div().text_color(t.fg_muted).child("Where its data comes from"));
                for (name, origin) in origins {
                    let id = origin.clone();
                    out = out.child(
                        div()
                            .id(SharedString::from(format!("origin-{name}")))
                            .cursor_pointer()
                            .text_color(t.graph_in)
                            .child(format!(
                                "{name} ← {}",
                                flow.nodes
                                    .iter()
                                    .find(|n| &n.id == origin)
                                    .map(|n| if n.kind == "param" {
                                        format!("input {}", n.label)
                                    } else {
                                        format!("line {}: {}", n.line, n.label)
                                    })
                                    .unwrap_or_else(|| origin.clone())
                            ))
                            .on_click(cx.listener(move |p, _, _, cx| p.select(id.clone(), cx))),
                    );
                }
            }
            let names =
                if n.kind == "return" && n.sets.is_empty() { vec!["(returned)".into()] } else { n.sets.clone() };
            for (i, edge) in flow.edges.iter().filter(|e| e.from == n.id).enumerate() {
                let id = edge.to.clone();
                out = out.child(
                    div()
                        .id(("pipeline-used-by", i))
                        .cursor_pointer()
                        .text_color(t.graph_out)
                        .child(format!(
                            "{} → {}",
                            edge.name,
                            flow.nodes
                                .iter()
                                .find(|n| n.id == edge.to)
                                .map(|n| format!("line {}: {}", n.line, n.label))
                                .unwrap_or_else(|| id.clone())
                        ))
                        .on_click(cx.listener(move |p, _, _, cx| p.select(id.clone(), cx))),
                );
            }
            for name in names {
                if let Some(s) = self.value(n, &name) {
                    out = out.child(self.summary_view(name, &s, cx));
                }
            }
        } else {
            out=out.child(self.raw.as_ref().map(|r|r.doc.clone()).unwrap_or_default()).child("Select a step to inspect its values and data dependencies. Run with data captures the selected function when the program calls it.");
        }
        if let Some(io) = self.log.io(&self.func_id()) {
            out = out.child(div().text_color(t.heat_2).child(format!("Function inputs / output · {} calls", io.total)));
            if let Some(call) = io.calls.first() {
                for (name, s) in &call.input {
                    out = out.child(self.summary_view(format!("Input: {name}"), s, cx));
                }
                if let Some(s) = &call.output {
                    out = out.child(self.summary_view("Returned".into(), s, cx));
                }
            }
        }
        out.into_any_element()
    }
}
impl Render for PipelinePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let actions = [
            (
                "pipeline-function",
                if self.func.is_empty() { "Pick function…".to_string() } else { format!("{}() ▾", self.func) },
                PipelineEvent::PickFunction,
            ),
            ("pipeline-refresh", "Refresh".into(), PipelineEvent::Refresh),
            ("pipeline-run", "▶ Run with data".into(), PipelineEvent::Run),
            ("pipeline-add", "+ Block".into(), PipelineEvent::AddBlock(false)),
            ("pipeline-each", "For each…".into(), PipelineEvent::AddBlock(true)),
            ("pipeline-new", "New pipeline…".into(), PipelineEvent::New),
            ("pipeline-export", "Export…".into(), PipelineEvent::Export),
        ];
        div()
            .id("pipeline-panel")
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .bg(t.bg_panel)
            .text_color(t.fg)
            .text_size(t.px(12.))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .gap(t.px(12.))
                    .p(t.px(8.))
                    .border_b_1()
                    .border_color(t.border)
                    .children(actions.into_iter().map(|(id, label, event)| {
                        div()
                            .id(id)
                            .debug_selector(move || id.into())
                            .cursor_pointer()
                            .text_color(t.heat_2)
                            .child(label)
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone())))
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .when(self.flow.is_some(), |d| {
                        d.child(
                            uniform_list(
                                "pipeline-steps",
                                self.flow.as_ref().map_or(0, |f| f.nodes.len()),
                                cx.processor(|p, range: std::ops::Range<usize>, _, cx| {
                                    range.map(|i| p.row(i, cx)).collect()
                                }),
                            )
                            .track_scroll(&self.scroll)
                            .flex_1()
                            .min_w_0()
                            .min_h_0(),
                        )
                        .child(self.details(cx))
                    })
                    .when(self.flow.is_none(), |d| {
                        d.p(t.px(12.)).child("Open a Python file and choose a function to inspect its data flow.")
                    }),
            )
            .child(div().flex_none().p(t.px(6.)).text_size(t.px(10.)).text_color(t.fg_muted).child(if self.loading {
                "Reading function…".to_string()
            } else if !self.status.is_empty() {
                self.status.clone()
            } else {
                format!(
                    "{} steps · {} dependencies{}",
                    self.flow.as_ref().map_or(0, |f| f.nodes.len()),
                    self.edges.len(),
                    if self.log.dropped > 0 { " · capture limit reached" } else { "" }
                )
            }))
    }
}
