//! Native project graph canvas and inspection, replacing `graph/graphView.ts`.
//! Layout and view construction run on a worker; rendering culls nodes and
//! edges outside the viewport and shares prepared geometry between frames.

use crate::theme::ForgeTheme;
use forge_core::{
    graph_layout::{layout_graph, Layout, LayoutEdge, LayoutNode, LayoutOptions, Placed},
    graph_view::{neighbourhood, GraphMode, GraphViewData, ViewOptions},
    live_run::{plan_frame, RunFrame, RunView, TraceTick},
    profile::{format_ms, heat_level, time_path, view_times, FnTime, RunProfile},
    project_graph::ProjectGraph,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    canvas, div, point, prelude::FluentBuilder, px, AnyElement, AppContext, Context, Entity, EventEmitter,
    InteractiveElement, IntoElement, MouseButton, ParentElement, PathBuilder, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window,
};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone)]
pub enum GraphEvent {
    AddApi(Option<String>),
    Refresh,
    View(ViewOptions),
    Open { module: String, line: usize },
    Box(String),
    Attach { box_id: String, module: String },
}
#[derive(Clone)]
pub struct DraggedSource {
    pub module: String,
}
struct SourceGhost(String);
impl Render for SourceGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        div().px(t.px(8.)).py(t.px(4.)).bg(t.raised).text_color(t.heat_2).child(format!("Connect {} to an API", self.0))
    }
}
#[derive(Clone)]
struct DrawEdge {
    from: String,
    to: String,
    a: Placed,
    b: Placed,
    cyclic: bool,
}
#[derive(Clone)]
struct NetFlash {
    spark: Arc<forge_core::live_run::NetSpark>,
    expires: std::time::Instant,
}
pub struct GraphPanel {
    pub graph: Arc<ProjectGraph>,
    pub data: Arc<GraphViewData>,
    layout: Arc<Layout>,
    edges: Arc<Vec<DrawEdge>>,
    pub options: ViewOptions,
    pub selected: Option<String>,
    outgoing: Arc<BTreeSet<String>>,
    incoming: Arc<BTreeSet<String>>,
    pub status: String,
    pub scanning: bool,
    zoom: f32,
    pan: (f32, f32),
    drag: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    query: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    pub profile: RunProfile,
    times: Arc<std::collections::BTreeMap<String, FnTime>>,
    max_self: f64,
    run_view: RunView,
    frame: Arc<RunFrame>,
    run_active: bool,
    replay: Vec<TraceTick>,
    replay_index: Option<usize>,
    last_tick: Option<TraceTick>,
    spark_phase: f32,
    animation: Option<gpui_kit::Task<()>>,
    playback: Option<gpui_kit::Task<()>>,
    net_flashes: Arc<Vec<NetFlash>>,
    net_animation: Option<gpui_kit::Task<()>>,
    net_phase: f32,
    testing_boxes: Arc<BTreeSet<String>>,
    test_animation: Option<gpui_kit::Task<()>>,
    view_size: std::rc::Rc<std::cell::Cell<gpui_kit::Size<gpui_kit::Pixels>>>,
}
impl EventEmitter<GraphEvent> for GraphPanel {}
pub fn prepare_layout(data: &GraphViewData) -> Layout {
    let nodes: Vec<_> =
        data.nodes.iter().map(|n| LayoutNode { id: n.id.clone(), width: 220.0, height: 58.0 }).collect();
    let edges: Vec<_> = data.edges.iter().map(|e| LayoutEdge { from: e.from.clone(), to: e.to.clone() }).collect();
    layout_graph(&nodes, &edges, LayoutOptions::default())
}
impl GraphPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter graph…"));
        let sub = cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.options.query = this.query.read(cx).value().to_string();
                cx.emit(GraphEvent::View(this.options.clone()));
            }
        });
        Self {
            graph: Arc::default(),
            data: Arc::default(),
            layout: Arc::default(),
            edges: Arc::default(),
            options: ViewOptions::default(),
            selected: None,
            outgoing: Arc::default(),
            incoming: Arc::default(),
            status: "Open a project folder to analyse its Python code.".into(),
            scanning: false,
            zoom: 1.0,
            pan: (18.0, 18.0),
            drag: None,
            query,
            _subscriptions: vec![sub],
            profile: RunProfile::default(),
            times: Arc::default(),
            max_self: 0.0,
            run_view: RunView::default(),
            frame: Arc::default(),
            run_active: false,
            replay: Vec::new(),
            replay_index: None,
            last_tick: None,
            spark_phase: 0.0,
            animation: None,
            playback: None,
            net_flashes: Arc::default(),
            net_animation: None,
            net_phase: 0.,
            testing_boxes: Arc::default(),
            test_animation: None,
            view_size: std::rc::Rc::new(std::cell::Cell::new(gpui_kit::size(px(0.0), px(0.0)))),
        }
    }
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.testing_boxes = Arc::default();
        self.test_animation = None;
        self.begin_run(cx);
        self.end_run(cx);
        self.graph = Arc::default();
        self.data = Arc::default();
        self.layout = Arc::default();
        self.edges = Arc::default();
        self.selected = None;
        self.options.focus = None;
        self.pan = (18.0, 18.0);
        self.status = "Ready to analyse this project.".into();
        cx.notify();
    }
    pub fn set_view(&mut self, graph: Arc<ProjectGraph>, data: GraphViewData, layout: Layout, cx: &mut Context<Self>) {
        self.edges = Arc::new(
            data.edges
                .iter()
                .filter_map(|e| {
                    Some(DrawEdge {
                        from: e.from.clone(),
                        to: e.to.clone(),
                        a: *layout.nodes.get(&e.from)?,
                        b: *layout.nodes.get(&e.to)?,
                        cyclic: e.cyclic,
                    })
                })
                .collect(),
        );
        self.graph = graph;
        self.run_view = RunView::new(self.options.mode, &data);
        self.data = Arc::new(data);
        self.layout = Arc::new(layout);
        self.scanning = false;
        if self.selected.as_ref().is_some_and(|id| !self.layout.nodes.contains_key(id)) {
            self.selected = None;
        }
        self.highlight();
        self.times = Arc::new(view_times(&self.profile, self.options.mode));
        self.max_self = self.times.values().map(|t| t.self_ms).fold(0.0_f64, f64::max);
        if self.run_active {
            if let Some(tick) = &self.last_tick {
                self.frame = Arc::new(plan_frame(tick, &self.run_view));
            }
        }
        cx.notify();
    }
    pub fn begin_run(&mut self, cx: &mut Context<Self>) {
        self.net_flashes = Arc::default();
        self.net_animation = None;
        self.profile = RunProfile::default();
        self.times = Arc::default();
        self.max_self = 0.0;
        self.frame = Arc::default();
        self.replay.clear();
        self.replay_index = None;
        self.last_tick = None;
        self.animation = None;
        self.playback = None;
        self.run_active = true;
        cx.notify();
    }
    pub fn attach_boxes(&mut self, boxes: &[forge_core::boxes::ApiBox], cx: &mut Context<Self>) {
        self.run_view.attach_boxes(boxes);
        if let Some(tick) = &self.last_tick {
            self.frame = Arc::new(plan_frame(tick, &self.run_view));
        }
        cx.notify();
    }
    pub fn end_run(&mut self, cx: &mut Context<Self>) {
        self.run_active = false;
        self.frame = Arc::default();
        self.animation = None;
        cx.notify();
    }
    pub fn set_profile(&mut self, profile: RunProfile, cx: &mut Context<Self>) {
        self.times = Arc::new(view_times(&profile, self.options.mode));
        self.max_self = self.times.values().map(|t| t.self_ms).fold(0.0_f64, f64::max);
        self.profile = profile;
        cx.notify();
    }
    pub fn network_record(&mut self, record: &forge_core::network::NetRecord, visible: bool, cx: &mut Context<Self>) {
        if !visible {
            return;
        }
        let Some(spark) = forge_core::live_run::plan_net_spark(record, &self.run_view) else { return };
        let flashes = Arc::make_mut(&mut self.net_flashes);
        let now = std::time::Instant::now();
        flashes.retain(|f| f.expires > now && !(f.spark.from == spark.from && f.spark.to == spark.to));
        if flashes.len() >= 32 {
            flashes.remove(0);
        }
        flashes.push(NetFlash { spark: Arc::new(spark), expires: now + std::time::Duration::from_millis(800) });
        self.net_phase = 0.;
        let animate =
            cx.try_global::<crate::app_state::AppState>().is_none_or(|s| s.settings.animations) && !cx.reduce_motion();
        if self.net_animation.is_none() {
            self.net_animation = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(std::time::Duration::from_millis(if animate { 16 } else { 800 })).await;
                if this
                    .update(cx, |p, cx| {
                        Arc::make_mut(&mut p.net_flashes).retain(|f| f.expires > std::time::Instant::now());
                        if p.net_flashes.is_empty() {
                            p.net_animation = None;
                            cx.notify();
                            return false;
                        }
                        if animate {
                            p.net_phase = (p.net_phase + 0.04) % 1.;
                        }
                        cx.notify();
                        true
                    })
                    .ok()
                    != Some(true)
                {
                    break;
                }
            }));
        }
        cx.notify();
    }
    pub fn box_testing(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        let id = format!("box:{id}");
        if on {
            Arc::make_mut(&mut self.testing_boxes).insert(id);
        } else {
            Arc::make_mut(&mut self.testing_boxes).remove(&id);
        }
        if self.testing_boxes.is_empty() {
            self.test_animation = None;
        } else if self.test_animation.is_none()
            && cx.try_global::<crate::app_state::AppState>().is_none_or(|s| s.settings.animations)
            && !cx.reduce_motion()
        {
            self.test_animation = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
                if this
                    .update(cx, |p, cx| {
                        if p.testing_boxes.is_empty() {
                            p.test_animation = None;
                            return false;
                        }
                        p.spark_phase = (p.spark_phase + 0.04) % 1.;
                        cx.notify();
                        true
                    })
                    .ok()
                    != Some(true)
                {
                    break;
                }
            }));
        }
        cx.notify();
    }
    pub fn tick(&mut self, tick: TraceTick, visible: bool, cx: &mut Context<Self>) {
        if self.replay.len() < 6000 {
            self.replay.push(tick.clone());
        }
        self.frame = Arc::new(plan_frame(&tick, &self.run_view));
        self.last_tick = Some(tick);
        self.spark_phase = 0.0;
        let animate = visible
            && cx.try_global::<crate::app_state::AppState>().is_none_or(|s| s.settings.animations)
            && !cx.reduce_motion();
        if !animate {
            self.animation = None;
        }
        if animate
            && self.animation.is_none()
            && self.run_active
            && (!self.frame.sparks.is_empty() || !self.frame.boxes.is_empty())
        {
            self.animation = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
                if this
                    .update(cx, |p, cx| {
                        if !p.run_active || (p.frame.sparks.is_empty() && p.frame.boxes.is_empty()) {
                            p.animation = None;
                            return false;
                        }
                        p.spark_phase = (p.spark_phase + 0.06) % 1.0;
                        cx.notify();
                        true
                    })
                    .ok()
                    != Some(true)
                {
                    break;
                }
            }));
        }
        cx.notify();
    }
    fn replay_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(tick) = self.replay.get(index) {
            self.replay_index = Some(index);
            self.frame = Arc::new(plan_frame(tick, &self.run_view));
            self.spark_phase = 0.5;
            cx.notify();
        }
    }
    fn highlight(&mut self) {
        let (out, inside) = self.selected.as_deref().map(|id| neighbourhood(&self.data.edges, id)).unwrap_or_default();
        self.outgoing = Arc::new(out);
        self.incoming = Arc::new(inside);
    }
    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        if let Some(n) = self.data.nodes.iter().find(|n| n.id == id && n.kind == "module") {
            self.options.focus = n.module.clone();
        }
        self.highlight();
        cx.notify();
    }
    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let modes = [
            ("graph-modules", "Modules", GraphMode::Modules),
            ("graph-calls", "Calls", GraphMode::Calls),
            ("graph-events", "Events", GraphMode::Events),
        ]
        .into_iter()
        .map(|(id, label, mode)| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px(t.px(7.0))
                .py(t.px(3.0))
                .cursor_pointer()
                .text_color(if self.options.mode == mode { t.heat_2 } else { t.fg_muted })
                .on_click(cx.listener(move |p, _, _, cx| {
                    p.options.mode = mode;
                    p.pan = (18.0, 18.0);
                    cx.emit(GraphEvent::View(p.options.clone()));
                }))
                .child(label)
        });
        let filters = [
            ("graph-packages", "Packages", self.options.show_external),
            ("graph-tests", "Tests", self.options.show_tests),
            ("graph-hosts", "Web APIs", self.options.show_hosts),
        ]
        .into_iter()
        .map(|(id, label, on)| {
            div()
                .id(id)
                .px(t.px(5.0))
                .py(t.px(3.0))
                .cursor_pointer()
                .text_color(if on { t.heat_2 } else { t.fg_muted })
                .on_click(cx.listener(move |p, _, _, cx| {
                    match id {
                        "graph-packages" => p.options.show_external = !p.options.show_external,
                        "graph-tests" => p.options.show_tests = !p.options.show_tests,
                        _ => p.options.show_hosts = !p.options.show_hosts,
                    };
                    cx.emit(GraphEvent::View(p.options.clone()));
                }))
                .child(label)
        });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(t.px(4.0))
            .p(t.px(5.0))
            .text_size(t.px(11.0))
            .border_b_1()
            .border_color(t.border)
            .children(modes)
            .children(filters)
            .child(div().min_w(t.px(100.0)).flex_1().child(Input::new(&self.query).appearance(false).cleanable(true)))
            .child(
                div()
                    .id("graph-refresh")
                    .px(t.px(6.0))
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GraphEvent::Refresh)))
                    .child("Refresh"),
            )
            .child(
                div()
                    .id("graph-fit")
                    .px(t.px(6.0))
                    .cursor_pointer()
                    .on_click(cx.listener(|p, _, window, cx| {
                        let measured = p.view_size.get();
                        let size = if measured.width > px(0.0) { measured } else { window.viewport_size() };
                        let width = p.layout.width.max(1.0) as f32;
                        let height = p.layout.height.max(1.0) as f32;
                        let density = cx.global::<ForgeTheme>().0.ui_scale;
                        p.zoom = ((f32::from(size.width) - 36.0) / (width * density))
                            .min((f32::from(size.height) - 36.0) / (height * density))
                            .clamp(0.2, 1.5);
                        p.pan = (18.0, 18.0);
                        cx.notify();
                    }))
                    .child("Fit"),
            )
            .when(!self.run_active && !self.replay.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .gap(t.px(5.0))
                        .child(div().id("graph-replay-prev").cursor_pointer().child("◀").on_click(
                            cx.listener(|p, _, _, cx| p.replay_at(p.replay_index.unwrap_or(1).saturating_sub(1), cx)),
                        ))
                        .child(div().id("graph-replay-play").cursor_pointer().child("Replay").on_click(cx.listener(
                            |p, _, _, cx| {
                                p.replay_at(0, cx);
                                p.playback = Some(cx.spawn(async move |this, cx| loop {
                                    cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                                    if this
                                        .update(cx, |p, cx| {
                                            let next = p.replay_index.unwrap_or(0) + 1;
                                            if next >= p.replay.len() {
                                                p.playback = None;
                                                return false;
                                            }
                                            p.replay_at(next, cx);
                                            true
                                        })
                                        .ok()
                                        != Some(true)
                                    {
                                        break;
                                    }
                                }));
                            },
                        )))
                        .child(div().id("graph-replay-next").cursor_pointer().child("▶").on_click(cx.listener(
                            |p, _, _, cx| {
                                p.replay_at((p.replay_index.unwrap_or(0) + 1).min(p.replay.len().saturating_sub(1)), cx)
                            },
                        )))
                        .child(div().id("graph-replay-clear").cursor_pointer().child("End replay").on_click(
                            cx.listener(|p, _, _, cx| {
                                p.playback = None;
                                p.replay_index = None;
                                p.frame = Arc::default();
                                cx.notify();
                            }),
                        )),
                )
            })
            .when(self.options.focus.is_some(), |d| {
                d.child(
                    div()
                        .id("graph-whole")
                        .cursor_pointer()
                        .px(t.px(6.0))
                        .on_click(cx.listener(|p, _, _, cx| {
                            p.options.focus = None;
                            cx.emit(GraphEvent::View(p.options.clone()));
                        }))
                        .child("Whole project"),
                )
            })
            .into_any_element()
    }
    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let node = self.selected.as_ref().and_then(|id| self.data.nodes.iter().find(|n| &n.id == id))?;
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut rows = vec![
            div().text_color(t.heat_2).child(node.label.clone()).into_any_element(),
            div().text_color(t.fg_muted).child(node.detail.clone()).into_any_element(),
        ];
        if node.unused {
            rows.push(
                div()
                    .text_color(t.warn)
                    .child("Nothing else in the project seems to use this symbol.")
                    .into_any_element(),
            );
        }
        if node.cyclic {
            for cycle in self.graph.cycles.iter().filter(|c| node.module.as_ref().is_some_and(|m| c.contains(m))) {
                rows.push(
                    div().text_color(t.err).child(format!("Import cycle: {}", cycle.join(" → "))).into_any_element(),
                );
            }
        }
        if let Some(time) = self.times.get(&node.id) {
            rows.push(
                div()
                    .text_color(t.warn)
                    .child(format!(
                        "{} total · {} own · {} calls",
                        format_ms(time.total_ms),
                        format_ms(time.self_ms),
                        time.calls
                    ))
                    .into_any_element(),
            );
            for id in time_path(&self.profile, &node.id) {
                rows.push(div().text_color(t.fg_muted).child(id).into_any_element());
            }
        }
        if let Some(module) = node.module.clone() {
            let line = node.line.unwrap_or(0);
            rows.push(
                div()
                    .id("graph-open-source")
                    .debug_selector(|| "graph-open-source".into())
                    .cursor_pointer()
                    .text_color(t.link)
                    .on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(GraphEvent::Open { module: module.clone(), line })),
                    )
                    .child("Open source")
                    .into_any_element(),
            );
            if let Some(m) = self.graph.modules.iter().find(|m| m.id == module_of_node(node)) {
                if let Some(generated) = &m.generated {
                    rows.push(
                        div()
                            .debug_selector(|| "graph-generated-state".into())
                            .text_color(if generated["edited"] == true { t.warn } else { t.fg_muted })
                            .child(format!(
                                "OpenAPI client: {}{}",
                                generated["slug"].as_str().unwrap_or("API"),
                                if generated["edited"] == true {
                                    " · edited by hand"
                                } else {
                                    " · unchanged since generation"
                                }
                            ))
                            .into_any_element(),
                    );
                }
                if let Some(error) = &m.error {
                    rows.push(div().text_color(t.err).child(error.clone()).into_any_element());
                }
                for r in &m.routes {
                    rows.push(div().child(format!("{} {}", r.methods.join(" "), r.path)).into_any_element());
                }
                for env in &m.env {
                    rows.push(div().child(format!("Environment: {}", env.name)).into_any_element());
                }
                for u in self.graph.unreachable.iter().filter(|u| u.module == m.id) {
                    rows.push(
                        div()
                            .text_color(t.warn)
                            .child(forge_core::project_graph::describe_unreachable(&u.raw))
                            .into_any_element(),
                    );
                }
            }
        }
        if node.kind == "event" {
            for u in self.graph.events.iter().filter(|u| u.event.as_deref() == Some(node.label.as_str())) {
                let module = u.module.clone();
                let line = u.line.saturating_sub(1);
                rows.push(
                    div()
                        .id(SharedString::from(format!("graph-event-{}-{}", u.func, u.line)))
                        .cursor_pointer()
                        .text_color(t.link)
                        .on_click(
                            cx.listener(move |_, _, _, cx| cx.emit(GraphEvent::Open { module: module.clone(), line })),
                        )
                        .child(format!(
                            "{}: {} ({})",
                            if u.kind == "handles" { "Handled by" } else { "Emitted by" },
                            u.func,
                            u.via
                        ))
                        .into_any_element(),
                );
            }
        }
        for (label, ids) in [("Uses", &self.outgoing), ("Used by", &self.incoming)] {
            if !ids.is_empty() {
                rows.push(div().text_color(t.fg_muted).child(label).into_any_element());
            }
            for id in ids.iter() {
                let id = id.clone();
                let label =
                    self.data.nodes.iter().find(|n| n.id == id).map(|n| n.label.clone()).unwrap_or_else(|| id.clone());
                rows.push(
                    div()
                        .id(SharedString::from(format!("graph-neighbour-{label}")))
                        .cursor_pointer()
                        .on_click(cx.listener(move |p, _, _, cx| p.select(id.clone(), cx)))
                        .child(label)
                        .into_any_element(),
                );
            }
        }
        Some(
            div()
                .id("graph-details")
                .w(t.px(230.0))
                .flex_none()
                .h_full()
                .overflow_y_scroll()
                .p(t.px(10.0))
                .flex()
                .flex_col()
                .gap(t.px(7.0))
                .text_size(t.px(11.0))
                .border_l_1()
                .border_color(t.border)
                .children(rows)
                .into_any_element(),
        )
    }
}
fn module_of_node(n: &forge_core::graph_view::ViewNode) -> &str {
    n.module.as_deref().unwrap_or("")
}
impl Render for GraphPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let scale = self.zoom * t.ui_scale;
        let pan = self.pan;
        let measured = self.view_size.get();
        let viewport = if measured.width > px(0.0) { measured } else { window.viewport_size() };
        let visible = |p: &Placed| {
            let x = p.x as f32 * scale + pan.0;
            let y = p.y as f32 * scale + pan.1;
            x + p.width as f32 * scale >= 0.0
                && y + p.height as f32 * scale >= 0.0
                && x <= f32::from(viewport.width)
                && y <= f32::from(viewport.height)
        };
        let nodes: Vec<_> = self
            .data
            .nodes
            .iter()
            .filter_map(|n| {
                let p = self.layout.nodes.get(&n.id)?;
                if !visible(p) {
                    return None;
                }
                let id = n.id.clone();
                let click_id = id.clone();
                let module = n.module.clone();
                let drag_module = module.clone();
                let add_module = module.clone();
                let drop_id = id.clone();
                let line = n.line.unwrap_or(0);
                let heat = self.times.get(&id).map_or(0, |time| heat_level(time.self_ms, self.max_self));
                let net =
                    self.net_flashes.iter().rev().find(|f| f.spark.from == id || f.spark.to.as_ref() == Some(&id));
                let border = if let Some(flash) = net {
                    if flash.spark.ok {
                        t.ok
                    } else {
                        t.err
                    }
                } else if self.frame.glow.as_ref() == Some(&id)
                    || self.frame.boxes.contains(&id)
                    || self.testing_boxes.contains(&id)
                    || self.selected.as_ref() == Some(&id)
                {
                    t.heat_2
                } else if self.outgoing.contains(&id) {
                    t.graph_out
                } else if self.incoming.contains(&id) {
                    t.graph_in
                } else if n.error || n.cyclic {
                    t.err
                } else if n.unused {
                    t.warn
                } else if heat >= 3 {
                    t.heat_3
                } else if heat >= 2 {
                    t.heat_2
                } else if heat >= 1 || n.kind == "box" {
                    t.heat_1
                } else {
                    t.line_strong
                };
                let selector = format!("graph-node-{id}");
                Some(
                    div()
                        .id(SharedString::from(id))
                        .debug_selector(move || selector.clone())
                        .absolute()
                        .left(px(p.x as f32 * scale + pan.0))
                        .top(px(p.y as f32 * scale + pan.1))
                        .w(px(p.width as f32 * scale))
                        .h(px(p.height as f32 * scale))
                        .border_1()
                        .border_color(border)
                        .bg(if n.matches { t.bg_active } else { t.raised })
                        .rounded(px(3.0 * scale))
                        .p(px(8.0 * scale))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .cursor_pointer()
                        .when_some(drag_module, |node, module| {
                            node.on_drag(DraggedSource { module: module.clone() }, move |_, _, _, cx| {
                                cx.new(|_| SourceGhost(module.clone()))
                            })
                        })
                        .on_drop(cx.listener(move |_, source: &DraggedSource, _, cx| {
                            if let Some(id) = drop_id.strip_prefix("box:") {
                                cx.emit(GraphEvent::Attach { box_id: id.into(), module: source.module.clone() });
                                cx.stop_propagation();
                            }
                        }))
                        .on_drop(cx.listener(move |_, _: &crate::panels::connect::ApiBoxDrag, _, cx| {
                            cx.emit(GraphEvent::AddApi(add_module.clone()));
                            cx.stop_propagation();
                        }))
                        .on_click(cx.listener(move |p, e: &gpui_kit::ClickEvent, _, cx| {
                            if let Some(id) = click_id.strip_prefix("box:") {
                                cx.emit(GraphEvent::Box(id.into()));
                                return;
                            }
                            p.select(click_id.clone(), cx);
                            if e.click_count() >= 2 {
                                if let Some(module) = &module {
                                    cx.emit(GraphEvent::Open { module: module.clone(), line });
                                }
                            }
                        }))
                        .child(
                            div()
                                .text_size(px(12.0 * scale))
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(n.label.clone()),
                        )
                        .when(n.box_rows.is_empty(), |node| {
                            node.child(
                                div()
                                    .text_size(px(10.0 * scale))
                                    .text_color(t.fg_muted)
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(
                                        self.times
                                            .get(&n.id)
                                            .map(|time| format!("{} · {} own", n.detail, format_ms(time.self_ms)))
                                            .unwrap_or_else(|| n.detail.clone()),
                                    ),
                            )
                        })
                        .children(n.box_rows.iter().map(|row| {
                            div()
                                .h(px(18. * scale))
                                .flex_none()
                                .flex()
                                .gap(px(5. * scale))
                                .text_size(px(10. * scale))
                                .child(
                                    div()
                                        .text_color(match row.ok {
                                            None => t.fg_muted,
                                            Some(true) => t.ok,
                                            Some(false) => t.err,
                                        })
                                        .child("●"),
                                )
                                .child(div().min_w_0().text_ellipsis().whitespace_nowrap().child(row.text.clone()))
                        })),
                )
            })
            .collect();
        let edges = self.edges.clone();
        let selected = self.selected.clone();
        let paint_theme = t.clone();
        let frame = self.frame.clone();
        let phase = self.spark_phase;
        let measurement = self.view_size.clone();
        let flashes = self.net_flashes.clone();
        let net_phase = self.net_phase;
        let layout = self.layout.clone();
        let testing = self.testing_boxes.clone();
        let background = canvas(
            move |bounds, _, _| {
                measurement.set(bounds.size);
            },
            move |bounds, _, window, _| {
                let convert = |x: f64, y: f64| {
                    point(
                        bounds.origin.x + px(x as f32 * scale + pan.0),
                        bounds.origin.y + px(y as f32 * scale + pan.1),
                    )
                };
                for e in edges.iter() {
                    let (a, b) = (&e.a, &e.b);
                    let min_x = a.x.min(b.x) as f32 * scale + pan.0;
                    let max_x = (a.x + a.width).max(b.x + b.width) as f32 * scale + pan.0;
                    let min_y = a.y.min(b.y) as f32 * scale + pan.1;
                    let max_y = (a.y + a.height).max(b.y + b.height) as f32 * scale + pan.1;
                    if max_x < 0.0
                        || max_y < 0.0
                        || min_x > f32::from(bounds.size.width)
                        || min_y > f32::from(bounds.size.height)
                    {
                        continue;
                    }
                    let from = convert(a.x + a.width, a.y + a.height / 2.0);
                    let to = convert(b.x, b.y + b.height / 2.0);
                    let bend = px(((b.x - a.x - a.width).abs() / 2.0).max(30.0) as f32 * scale);
                    let forward = frame.boxes.contains(&e.to)
                        || testing.contains(&e.to)
                        || frame.sparks.iter().any(|(from, to)| from == &e.from && to == &e.to);
                    let reverse = frame.sparks.iter().any(|(from, to)| from == &e.to && to == &e.from);
                    let color = if forward || reverse {
                        paint_theme.heat_2
                    } else if e.cyclic {
                        paint_theme.err
                    } else if selected.as_ref() == Some(&e.from) {
                        paint_theme.graph_out
                    } else if selected.as_ref() == Some(&e.to) {
                        paint_theme.graph_in
                    } else {
                        paint_theme.line_strong
                    };
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(from);
                    path.cubic_bezier_to(point(from.x + bend, from.y), point(to.x - bend, to.y), to);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                    let mut arrow = PathBuilder::stroke(px(1.5));
                    arrow.move_to(point(to.x - px(5.0), to.y - px(3.0)));
                    arrow.line_to(to);
                    arrow.line_to(point(to.x - px(5.0), to.y + px(3.0)));
                    if let Ok(path) = arrow.build() {
                        window.paint_path(path, color);
                    }
                    if forward || reverse {
                        let progress = if reverse && !forward { 1.0 - phase } else { phase };
                        let q = 1.0 - progress;
                        let x = from.x * (q * q * q)
                            + (from.x + bend) * (3.0 * q * q * progress)
                            + (to.x - bend) * (3.0 * q * progress * progress)
                            + to.x * (progress * progress * progress);
                        let y = from.y * (q * q * q)
                            + from.y * (3.0 * q * q * progress)
                            + to.y * (3.0 * q * progress * progress)
                            + to.y * (progress * progress * progress);
                        window.paint_quad(gpui_kit::fill(
                            gpui_kit::Bounds::new(point(x - px(2.0), y - px(2.0)), gpui_kit::size(px(4.0), px(4.0))),
                            paint_theme.heat_2,
                        ));
                    }
                }
                for flash in flashes.iter() {
                    let Some(a) = layout.nodes.get(&flash.spark.from) else { continue };
                    let Some(b) = flash.spark.to.as_ref().and_then(|id| layout.nodes.get(id)) else { continue };
                    let from = convert(a.x + a.width, a.y + a.height / 2.);
                    let to = convert(b.x, b.y + b.height / 2.);
                    let bend = px(((b.x - a.x - a.width).abs() / 2.).max(30.) as f32 * scale);
                    let color = if flash.spark.ok { paint_theme.ok } else { paint_theme.err };
                    let mut path = PathBuilder::stroke(px(2.));
                    path.move_to(from);
                    path.cubic_bezier_to(point(from.x + bend, from.y), point(to.x - bend, to.y), to);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color)
                    }
                    let p = net_phase;
                    let q = 1. - p;
                    let x = from.x * (q * q * q)
                        + (from.x + bend) * (3. * q * q * p)
                        + (to.x - bend) * (3. * q * p * p)
                        + to.x * (p * p * p);
                    let y =
                        from.y * (q * q * q) + from.y * (3. * q * q * p) + to.y * (3. * q * p * p) + to.y * (p * p * p);
                    window.paint_quad(gpui_kit::fill(
                        gpui_kit::Bounds::new(point(x - px(3.), y - px(3.)), gpui_kit::size(px(6.), px(6.))),
                        color,
                    ));
                }
            },
        )
        .absolute()
        .size_full();
        let body = div()
            .id("graph-canvas")
            .debug_selector(|| "graph-canvas".into())
            .on_drop(cx.listener(|_, _: &crate::panels::connect::ApiBoxDrag, _, cx| {
                cx.emit(GraphEvent::AddApi(None));
                cx.stop_propagation();
            }))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(background)
            .children(nodes)
            .on_scroll(cx.listener(|p, e: &gpui_kit::ScrollEvent, _, cx| {
                let delta = e.delta.pixel_delta(px(20.0));
                if e.modifiers.control || e.modifiers.platform {
                    p.zoom = (p.zoom * (f32::from(delta.y) * 0.002).exp()).clamp(0.2, 2.5);
                } else {
                    p.pan.0 += f32::from(delta.x);
                    p.pan.1 += f32::from(delta.y);
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|p, e: &gpui_kit::MouseDownEvent, _, _| p.drag = Some(e.position)),
            )
            .on_mouse_move(cx.listener(|p, e: &gpui_kit::MouseMoveEvent, _, cx| {
                if e.pressed_button != Some(MouseButton::Middle) {
                    p.drag = None;
                    return;
                }
                if let Some(old) = p.drag {
                    p.drag = Some(e.position);
                    p.pan.0 += f32::from(e.position.x - old.x);
                    p.pan.1 += f32::from(e.position.y - old.y);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Middle, cx.listener(|p, _, _, _| p.drag = None));
        let status = if self.scanning {
            "Analysing project…".into()
        } else if let Some(note) = &self.data.note {
            note.clone()
        } else {
            format!(
                "{} nodes · {} connections{}",
                self.data.nodes.len(),
                self.data.edges.len(),
                if self.graph.truncated { " · scan limit reached" } else { "" }
            )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .text_color(t.fg)
            .bg(t.bg_panel)
            .child(self.controls(cx))
            .child(div().flex().flex_1().min_h_0().child(body).children(self.details(cx)))
            .child(
                div()
                    .px(t.px(8.0))
                    .py(t.px(4.0))
                    .text_size(t.px(10.0))
                    .text_color(t.fg_muted)
                    .child(if !self.status.is_empty() && !self.scanning { self.status.clone() } else { status }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext};
    #[gpui_kit::test]
    fn api_boxes_render_wire_activity_and_click_to_open_settings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(ForgeTheme(Arc::new(crate::theme::Theme::molten())));
        });
        let mut panel = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let p = cx.new(|cx| GraphPanel::new(window, cx));
            panel = Some(p.clone());
            gpui_kit::component::Root::new(p, window, cx)
        });
        let panel = panel.unwrap();
        let mut data = GraphViewData {
            nodes: vec![forge_core::graph_view::ViewNode {
                id: "api".into(),
                module: Some("api".into()),
                label: "api".into(),
                kind: "module".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut layout = prepare_layout(&data);
        let mut api = forge_core::boxes::new_api_box(&[], "Test");
        api.base_url = "https://api.test".into();
        api.attach = Some(forge_core::boxes::BoxAttach { module: "api".into() });
        let box_id = api.id.clone();
        forge_core::box_layer::add_boxes(&mut data, &mut layout, &[api.clone()], GraphMode::Modules);
        panel.update(cx, |p, cx| {
            p.set_view(Arc::default(), data, layout, cx);
            p.attach_boxes(&[api], cx);
            p.tick(TraceTick { at: Some("api:test_data".into()), ..Default::default() }, false, cx);
            assert!(p.frame.boxes.contains(&format!("box:{box_id}")));
            assert_eq!(p.edges.len(), 1);
            p.network_record(
                &forge_core::network::NetRecord {
                    func: Some("api:test_data".into()),
                    host: "api.test".into(),
                    status: Some(500),
                    ..Default::default()
                },
                true,
                cx,
            );
            assert_eq!(p.net_flashes.len(), 1);
            assert!(!p.net_flashes[0].spark.ok);
            assert_eq!(p.net_flashes[0].spark.to.as_deref(), Some("box:test-1"));
        });
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = opened.clone();
        let attached = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen_attached = attached.clone();
        let _sub = panel.update(cx, |_, cx| {
            cx.subscribe(&panel, move |_, _, event, _| {
                if let GraphEvent::Box(id) = event {
                    seen.borrow_mut().push(id.clone());
                }
                if let GraphEvent::Attach { box_id, module } = event {
                    seen_attached.borrow_mut().push((box_id.clone(), module.clone()));
                }
            })
        });
        cx.run_until_parked();
        assert_eq!(box_id, "test-1");
        let bounds = cx.debug_bounds("graph-node-box:test-1").expect("API box is visible beside module");
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), vec![box_id]);
        let source = cx.debug_bounds("graph-node-api").unwrap();
        cx.simulate_mouse_down(source.center(), MouseButton::Left, gpui_kit::Modifiers::default());
        cx.simulate_mouse_move(
            source.center() + point(px(12.), px(0.)),
            Some(MouseButton::Left),
            gpui_kit::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.simulate_mouse_move(bounds.center(), Some(MouseButton::Left), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        cx.simulate_mouse_up(bounds.center(), MouseButton::Left, gpui_kit::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(*attached.borrow(), vec![("test-1".into(), "api".into())]);
    }
    #[gpui_kit::test]
    fn graph_nodes_show_details_and_source_actions_use_zero_based_lines(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(crate::theme::ForgeTheme(std::sync::Arc::new(crate::theme::Theme::molten())));
        });
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../forge-core/tests/fixtures/project_graph.json")).unwrap();
        let mut graph: forge_core::project_graph::ProjectGraph =
            serde_json::from_value(fixture["expected"].clone()).unwrap();
        graph.modules.iter_mut().find(|m| m.id == "pkg.a").unwrap().generated =
            Some(serde_json::json!({"kind":"openapi","slug":"test","edited":true}));
        let mut panel = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let p = cx.new(|cx| GraphPanel::new(window, cx));
            panel = Some(p.clone());
            gpui_kit::component::Root::new(p, window, cx)
        });
        let panel = panel.unwrap();
        let options = forge_core::graph_view::ViewOptions::default();
        let data = forge_core::graph_view::build_view(&graph, &options);
        let layout = prepare_layout(&data);
        panel.update(cx, |p, cx| p.set_view(std::sync::Arc::new(graph), data, layout, cx));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("graph-node-pkg.a").expect("Visible module node");
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        panel.read_with(cx, |p, _| assert_eq!(p.selected.as_deref(), Some("pkg.a")));
        assert!(cx.debug_bounds("graph-open-source").is_some());
        assert!(cx.debug_bounds("graph-generated-state").is_some());
        assert_eq!(panel.read_with(cx, |p, _| p.data.nodes.len()), 2);
        let pan = panel.read_with(cx, |p, _| p.pan);
        cx.simulate_mouse_move(point(px(100.0), px(180.0)), None, gpui_kit::Modifiers::default());
        cx.simulate_mouse_move(point(px(140.0), px(200.0)), None, gpui_kit::Modifiers::default());
        assert_eq!(panel.read_with(cx, |p, _| p.pan), pan, "Hover must not pan the canvas");
        panel.update(cx, |p, _| p.drag = Some(point(px(80.), px(160.))));
        cx.simulate_mouse_move(point(px(100.), px(180.)), None, gpui_kit::Modifiers::default());
        assert_eq!(
            panel.read_with(cx, |p, _| p.pan),
            pan,
            "Releasing the middle button outside the canvas must end panning"
        );
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = opened.clone();
        let _sub = panel.update(cx, |_, cx| {
            cx.subscribe(&panel, move |_, _, event, cx| {
                if let GraphEvent::Open { module, line } = event {
                    observed.borrow_mut().push((module.clone(), *line));
                }
                let _ = cx;
            })
        });
        let source = cx.debug_bounds("graph-open-source").unwrap();
        cx.simulate_click(source.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), vec![("pkg.a".into(), 0)]);
        let _cx: &VisualTestContext = cx;
    }
}
