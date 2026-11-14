//! Shared native schema/object/recursion canvas: layout once, cull on paint.
use crate::theme::ForgeTheme;
use forge_core::graph_layout::{layout_graph, Layout, LayoutEdge, LayoutNode, LayoutOptions};
use gpui_kit::{
    canvas, div, point, px, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement, PathBuilder, Render,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
#[derive(Clone, Debug, Default)]
pub struct DiagramNode {
    pub id: String,
    pub title: String,
    pub fields: Vec<String>,
    pub source: Option<(String, usize)>,
    pub functions: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct DiagramEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}
#[derive(Clone)]
pub enum DiagramEvent {
    Open(String, usize),
    Function(String),
    Refresh,
}
pub struct DiagramPanel {
    pub title: String,
    pub status: String,
    pub nodes: Arc<Vec<DiagramNode>>,
    pub edges: Arc<Vec<DiagramEdge>>,
    pub selected: Option<String>,
    layout: Arc<Layout>,
    pan: (f32, f32),
    zoom: f32,
    viewport: Rc<Cell<gpui_kit::Size<gpui_kit::Pixels>>>,
}
impl EventEmitter<DiagramEvent> for DiagramPanel {}
impl DiagramPanel {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.into(),
            status: String::new(),
            nodes: Arc::default(),
            edges: Arc::default(),
            selected: None,
            layout: Arc::new(Layout::default()),
            pan: (10., 10.),
            zoom: 1.,
            viewport: Rc::new(Cell::new(gpui_kit::size(px(700.), px(500.)))),
        }
    }
    pub fn set_data(&mut self, nodes: Vec<DiagramNode>, edges: Vec<DiagramEdge>, cx: &mut Context<Self>) {
        let geometry = nodes
            .iter()
            .map(|n| LayoutNode { id: n.id.clone(), width: 230., height: 45. + n.fields.len().min(8) as f64 * 16. })
            .collect::<Vec<_>>();
        let links = edges.iter().map(|e| LayoutEdge { from: e.from.clone(), to: e.to.clone() }).collect::<Vec<_>>();
        self.layout = Arc::new(layout_graph(&geometry, &links, LayoutOptions::default()));
        self.nodes = Arc::new(nodes);
        self.edges = Arc::new(edges);
        if self.selected.as_ref().is_some_and(|id| !self.nodes.iter().any(|n| &n.id == id)) {
            self.selected = None;
        }
        self.fit();
        cx.notify();
    }
    fn fit(&mut self) {
        let size = self.viewport.get();
        self.zoom = ((f32::from(size.width) - 20.) / self.layout.width.max(1.) as f32)
            .min((f32::from(size.height) - 20.) / self.layout.height.max(1.) as f32)
            .clamp(0.15, 1.);
        self.pan = (10., 10.);
    }
}
impl Render for DiagramPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let layout = self.layout.clone();
        let edges = self.edges.clone();
        let selected = self.selected.clone();
        let (pan, zoom) = (self.pan, self.zoom);
        let viewport = self.viewport.clone();
        let theme = t.clone();
        let background = canvas(
            move |bounds, _, _| {
                viewport.set(bounds.size);
            },
            move |bounds, _, window, _| {
                for edge in edges.iter() {
                    let Some(a) = layout.nodes.get(&edge.from) else {
                        continue;
                    };
                    let Some(b) = layout.nodes.get(&edge.to) else {
                        continue;
                    };
                    let from = point(
                        bounds.left() + px(pan.0 + (a.x + a.width) as f32 * zoom),
                        bounds.top() + px(pan.1 + (a.y + a.height / 2.) as f32 * zoom),
                    );
                    let to = point(
                        bounds.left() + px(pan.0 + b.x as f32 * zoom),
                        bounds.top() + px(pan.1 + (b.y + b.height / 2.) as f32 * zoom),
                    );
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(from);
                    let bend = px(40. * zoom);
                    path.cubic_bezier_to(point(from.x + bend, from.y), point(to.x - bend, to.y), to);
                    let color = if selected.as_ref().is_some_and(|id| edge.from == *id || edge.to == *id) {
                        theme.heat_2
                    } else {
                        theme.line_strong
                    };
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                    let mut arrow = PathBuilder::stroke(px(1.5));
                    arrow.move_to(point(to.x - px(4.), to.y - px(3.)));
                    arrow.line_to(to);
                    arrow.line_to(point(to.x - px(4.), to.y + px(3.)));
                    if let Ok(path) = arrow.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .absolute()
        .size_full();
        let size = self.viewport.get();
        let nodes = self
            .nodes
            .iter()
            .filter_map(|n| {
                let p = self.layout.nodes.get(&n.id)?;
                let (x, y, w, h) = (
                    self.pan.0 + p.x as f32 * self.zoom,
                    self.pan.1 + p.y as f32 * self.zoom,
                    p.width as f32 * self.zoom,
                    p.height as f32 * self.zoom,
                );
                if x + w < 0. || y + h < 0. || x > f32::from(size.width) || y > f32::from(size.height) {
                    return None;
                }
                let id = n.id.clone();
                let debug = format!("diagram-node-{id}");
                Some(
                    div()
                        .id(SharedString::from(id.clone()))
                        .debug_selector(move || debug.clone())
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w))
                        .h(px(h))
                        .overflow_hidden()
                        .border_1()
                        .border_color(if self.selected.as_ref() == Some(&id) { t.heat_2 } else { t.line_strong })
                        .rounded(px(3.))
                        .p(px(6. * self.zoom))
                        .bg(t.raised)
                        .text_size(px(11. * self.zoom))
                        .cursor_pointer()
                        .child(div().text_color(t.heat_2).child(n.title.clone()))
                        .children(
                            n.fields.iter().take(8).map(|s| div().whitespace_nowrap().text_ellipsis().child(s.clone())),
                        )
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.selected = Some(id.clone());
                            cx.notify();
                        })),
                )
            })
            .collect::<Vec<_>>();
        let mut detail = div()
            .id("diagram-details")
            .w(t.px(250.))
            .h_full()
            .overflow_y_scroll()
            .p(t.px(10.))
            .flex()
            .flex_col()
            .gap(t.px(7.))
            .border_l_1()
            .border_color(t.border);
        if let Some(n) = self.selected.as_ref().and_then(|id| self.nodes.iter().find(|n| &n.id == id)) {
            detail = detail.child(n.title.clone()).children(n.fields.iter().map(|s| div().child(s.clone())));
            if let Some((path, line)) = &n.source {
                let path = path.clone();
                let line = *line;
                detail = detail.child(
                    div()
                        .id("diagram-source")
                        .debug_selector(|| "diagram-source".into())
                        .cursor_pointer()
                        .text_color(t.link)
                        .child("Open source")
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(DiagramEvent::Open(path.clone(), line)))),
                );
            }
            for (i, f) in n.functions.iter().enumerate() {
                let f = f.clone();
                let label = f.clone();
                detail = detail.child(
                    div()
                        .id(("diagram-function", i))
                        .cursor_pointer()
                        .text_color(t.link)
                        .child(label)
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(DiagramEvent::Function(f.clone())))),
                );
            }
            for (i, e) in self.edges.iter().filter(|e| e.from == n.id || e.to == n.id).enumerate() {
                let target = if e.from == n.id { e.to.clone() } else { e.from.clone() };
                let label = format!("{} {} {}", e.from, e.label, e.to);
                detail = detail.child(
                    div().id(("diagram-relation", i)).cursor_pointer().text_color(t.graph_in).child(label).on_click(
                        cx.listener(move |p, _, _, cx| {
                            p.selected = Some(target.clone());
                            cx.notify();
                        }),
                    ),
                );
            }
        } else {
            detail = detail.child("Select a node to inspect its fields and relationships.");
        }
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(t.bg_panel)
            .text_color(t.fg)
            .text_size(t.px(12.))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(t.px(15.))
                    .p(t.px(7.))
                    .child(self.title.clone())
                    .child(div().id("diagram-fit").cursor_pointer().text_color(t.link).child("Fit").on_click(
                        cx.listener(|p, _, _, cx| {
                            p.fit();
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .id("diagram-refresh")
                            .cursor_pointer()
                            .text_color(t.link)
                            .child("Refresh")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DiagramEvent::Refresh))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .id("diagram-canvas")
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .child(background)
                            .children(nodes)
                            .on_scroll(cx.listener(|p, e: &gpui_kit::ScrollEvent, _, cx| {
                                let delta = e.delta.pixel_delta(px(20.));
                                if e.modifiers.control || e.modifiers.platform {
                                    p.zoom = (p.zoom * (f32::from(delta.y) * 0.002).exp()).clamp(0.15, 2.5);
                                } else {
                                    p.pan.0 += f32::from(delta.x);
                                    p.pan.1 += f32::from(delta.y);
                                }
                                cx.notify();
                            })),
                    )
                    .child(detail),
            )
            .child(div().p(t.px(6.)).text_color(t.fg_muted).child(if self.status.is_empty() {
                format!(
                    "{} nodes · {} relationships · scroll to pan, Ctrl+scroll to zoom",
                    self.nodes.len(),
                    self.edges.len()
                )
            } else {
                self.status.clone()
            }))
    }
}
