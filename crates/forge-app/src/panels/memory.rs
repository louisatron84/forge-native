use crate::theme::ForgeTheme;
use forge_core::memory::{format_bytes, MemMessage, MemoryLog};
use gpui_kit::{
    canvas, div, point, px, uniform_list, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    PathBuilder, Render, StatefulInteractiveElement, Styled, Task, Window,
};
use std::{sync::Arc, time::Duration};
#[derive(Clone)]
pub enum MemoryEvent {
    Run,
    Open(String, u32),
}
#[derive(Clone)]
enum Row {
    Site(String, u32, String),
    Type(String, String),
    Kept(String),
}
pub struct MemoryPanel {
    pub graph: Arc<forge_core::project_graph::ProjectGraph>,
    pub log: MemoryLog,
    rows: Vec<Row>,
    mode: usize,
    selected: Option<String>,
    refresh: Option<Task<()>>,
}
impl EventEmitter<MemoryEvent> for MemoryPanel {}
impl MemoryPanel {
    pub fn new() -> Self {
        Self {
            graph: Arc::default(),
            log: MemoryLog::default(),
            rows: Vec::new(),
            mode: 0,
            selected: None,
            refresh: None,
        }
    }
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        *self = Self::new();
        cx.notify();
    }
    pub fn begin(&mut self, id: u64, cx: &mut Context<Self>) {
        self.log.begin(id);
        self.rows.clear();
        self.refresh = None;
        cx.notify();
    }
    pub fn add(&mut self, m: MemMessage, cx: &mut Context<Self>) {
        self.log.add(m);
        if self.refresh.is_none() {
            self.refresh = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(32)).await;
                this.update(cx, |p, cx| {
                    p.refresh = None;
                    p.rebuild();
                    cx.notify();
                })
                .ok();
            }));
        }
    }
    pub fn set_graph(&mut self, graph: Arc<forge_core::project_graph::ProjectGraph>, cx: &mut Context<Self>) {
        self.graph = graph;
        self.rebuild();
        cx.notify();
    }
    fn rebuild(&mut self) {
        self.rows = match self.mode {
            0 => self
                .log
                .sites
                .iter()
                .map(|s| {
                    let function = self
                        .graph
                        .modules
                        .iter()
                        .find(|m| m.path == s.path)
                        .and_then(|m| {
                            m.symbols
                                .iter()
                                .filter(|sym| sym.line <= s.line as usize && sym.end_line >= s.line as usize)
                                .min_by_key(|sym| sym.end_line - sym.line)
                                .map(|sym| format!("{}.{} · ", m.id, sym.qual))
                        })
                        .unwrap_or_default();
                    Row::Site(
                        s.path.clone(),
                        s.line,
                        format!(
                            "{function}{}:{} · {} · {} allocations",
                            s.path,
                            s.line,
                            format_bytes(s.bytes),
                            s.count
                        ),
                    )
                })
                .collect(),
            1 => self
                .log
                .types
                .keys()
                .map(|name| {
                    Row::Type(
                        name.clone(),
                        format!(
                            "{name} · {} live{}",
                            self.log.count(name),
                            if self.log.growing(name) { " · keeps growing" } else { "" }
                        ),
                    )
                })
                .collect(),
            _ => self
                .log
                .kept
                .iter()
                .map(|k| {
                    Row::Kept(format!(
                        "{} · {} retained · {}",
                        k.type_,
                        k.count,
                        if k.path.is_empty() { "root unknown".into() } else { k.path.join(" → ") }
                    ))
                })
                .collect(),
        };
    }
}
impl Render for MemoryPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let samples =
            Arc::new(self.selected.as_ref().and_then(|s| self.log.types.get(s)).unwrap_or(&self.log.samples).clone());
        let peak = samples.iter().map(|s| s.1).fold(1., f64::max);
        let end = samples.last().map_or(1., |s| s.0).max(0.001);
        let color = t.heat_2;
        let chart = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let mut path = PathBuilder::stroke(px(1.5));
                let stride = (samples.len() / f32::from(bounds.size.width).max(1.) as usize).max(1);
                for (i, (time, value)) in samples.iter().step_by(stride).enumerate() {
                    let p = point(
                        bounds.left() + bounds.size.width * (*time / end) as f32,
                        bounds.bottom() - bounds.size.height * (*value / peak) as f32,
                    );
                    if i == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            },
        )
        .w_full()
        .h(t.px(100.));
        div().size_full().flex().flex_col().min_h_0().bg(t.bg_panel).text_color(t.fg).text_size(t.px(12.))
            .child(div().flex().flex_wrap().gap(t.px(12.)).p(t.px(8.)).child(div().id("memory-run").debug_selector(||"memory-run".into()).cursor_pointer().text_color(t.heat_2).child("▶ Run with memory").on_click(cx.listener(|_,_,_,cx|cx.emit(MemoryEvent::Run)))).children(["Allocation sites","Live classes","Retained objects"].into_iter().enumerate().map(|(i,label)|div().id(("memory-mode",i)).cursor_pointer().child(label).on_click(cx.listener(move|p,_,_,cx|{p.mode=i;p.selected=None;p.rebuild();cx.notify();})))))
            .child(div().px(t.px(8.)).child(format!("Peak {} · {} samples{}{}",format_bytes(self.log.peak),self.log.samples.len(),if self.log.running{" · recording"}else{""},self.selected.as_ref().map(|s|format!(" · {s} live objects")).unwrap_or_default())))
            .child(chart).child(uniform_list("memory-records",self.rows.len(),cx.processor(|p,range:std::ops::Range<usize>,_,cx|{let theme=cx.global::<ForgeTheme>().0.clone();range.map(|i|{let row=p.rows[i].clone();let text=match &row{Row::Site(_,_,s)|Row::Type(_,s)|Row::Kept(s)=>s.clone()};div().id(("memory-row",i)).h(theme.px(28.)).px(theme.px(8.)).cursor_pointer().text_ellipsis().whitespace_nowrap().child(text).on_click(cx.listener(move|p,_,_,cx|match &row{Row::Site(path,line,_)=>cx.emit(MemoryEvent::Open(path.clone(),*line)),Row::Type(name,_)=>{p.selected=Some(name.clone());cx.notify();},_=>{}}))}).collect()})).flex_1().min_h_0())
            .child(div().p(t.px(7.)).text_size(t.px(10.)).text_color(t.fg_muted).child("Tracking is opt-in for this run and adds overhead. Retention paths explain what keeps objects alive; growth is a hint, not proof of a leak."))
    }
}
