//! Native runtime Network panel, replacing `renderer/networkView.ts`.
use crate::theme::ForgeTheme;
use forge_core::network::{
    compare_runs, filter_records, func_label, group_records, is_slow, ms_text, pretty_body, state_of, status_text,
    timeline_lanes, GroupBy, Lane, NetFilter, NetLog, NetRecord, NetState, Only,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    canvas, div, fill, point, px, size, uniform_list, AnyElement, AppContext, Bounds, Context, Entity, EventEmitter,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub enum NetworkEvent {
    OpenFunc(String),
}
#[derive(Clone)]
enum Row {
    Group(String),
    Record(Arc<NetRecord>),
    Compare(String),
}
pub struct NetworkPanel {
    pub log: NetLog,
    pub selected: Option<i64>,
    pub only: Only,
    pub group: GroupBy,
    pub comparing: bool,
    filter: Entity<InputState>,
    filtered: Vec<Arc<NetRecord>>,
    rows: Vec<Row>,
    timeline: Arc<Vec<(Lane, NetState)>>,
    timeline_end: f64,
    summary: String,
    running: bool,
    started: Instant,
    refresh_task: Option<Task<()>>,
    ticker: Option<Task<()>>,
    _subscription: Subscription,
}
impl EventEmitter<NetworkEvent> for NetworkPanel {}
impl NetworkPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter address, method, status, function…"));
        let subscription = cx.subscribe(&filter, |p, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                p.refresh(cx);
            }
        });
        Self {
            log: NetLog::default(),
            selected: None,
            only: Only::All,
            group: GroupBy::None,
            comparing: false,
            filter,
            filtered: Vec::new(),
            rows: Vec::new(),
            timeline: Arc::default(),
            timeline_end: 1.,
            summary: "Run with Network visible to capture requests and connections.".into(),
            running: false,
            started: Instant::now(),
            refresh_task: None,
            ticker: None,
            _subscription: subscription,
        }
    }
    pub fn begin(&mut self, id: u64, cx: &mut Context<Self>) {
        self.log.begin(id);
        self.selected = None;
        self.running = true;
        self.started = Instant::now();
        self.ticker = None;
        self.refresh(cx);
    }
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.log.clear();
        self.selected = None;
        self.running = false;
        self.ticker = None;
        self.refresh_task = None;
        self.refresh(cx);
    }
    pub fn end(&mut self, cx: &mut Context<Self>) {
        self.running = false;
        self.ticker = None;
        self.refresh(cx);
    }
    pub fn upsert(&mut self, record: Arc<NetRecord>, cx: &mut Context<Self>) {
        self.log.upsert(record);
        if self.refresh_task.is_none() {
            self.refresh_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(32)).await;
                this.update(cx, |p, cx| {
                    p.refresh_task = None;
                    p.refresh(cx);
                })
                .ok();
            }));
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let current = self.log.current();
        self.filtered =
            filter_records(&current, &NetFilter { text: self.filter.read(cx).value().to_string(), only: self.only });
        self.summary = format!(
            "{} requests · {} failed · {} slow{}",
            current.len(),
            current.iter().filter(|r| state_of(r) == NetState::Failed).count(),
            current.iter().filter(|r| is_slow(r)).count(),
            if self.log.dropped > 0 { " · capture limit reached" } else { "" }
        );
        self.rows = if self.comparing {
            compare_runs(self.log.previous(), &current)
                .into_iter()
                .map(|row| {
                    Row::Compare(format!(
                        "{} · {} · {} → {} requests · {} → {}",
                        row.change,
                        row.endpoint,
                        row.before.count,
                        row.after.count,
                        ms_text(Some(row.before.avg_ms)),
                        ms_text(Some(row.after.avg_ms))
                    ))
                })
                .collect()
        } else {
            let mut rows = Vec::new();
            for group in group_records(&self.filtered, self.group) {
                if !group.label.is_empty() {
                    rows.push(Row::Group(format!(
                        "{} · {} requests · {} failed · slowest {}",
                        group.label,
                        group.records.len(),
                        group.failed,
                        ms_text(Some(group.slowest))
                    )))
                }
                rows.extend(group.records.into_iter().map(Row::Record));
            }
            rows
        };
        let now = self.started.elapsed().as_secs_f64() * 1000.;
        let lanes = timeline_lanes(&self.filtered, now);
        self.timeline_end = lanes.iter().map(|l| l.end).fold(1., f64::max);
        self.timeline = Arc::new(
            lanes
                .into_iter()
                .map(|lane| {
                    let state = self.log.get(lane.id).map(|r| state_of(r)).unwrap_or(NetState::Pending);
                    (lane, state)
                })
                .collect(),
        );
        if self.running && self.filtered.iter().any(|r| r.ms.is_none()) && self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if this
                    .update(cx, |p, cx| {
                        if !p.running || !p.log.current().iter().any(|r| r.ms.is_none()) {
                            p.ticker = None;
                            return false;
                        }
                        p.refresh(cx);
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
    fn row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let base = div().h(t.px(26.)).px(t.px(7.)).text_size(t.px(11.)).text_ellipsis().whitespace_nowrap();
        match &self.rows[i] {
            Row::Group(text) => base.text_color(t.heat_2).child(text.clone()).into_any_element(),
            Row::Compare(text) => base.text_color(t.fg).child(text.clone()).into_any_element(),
            Row::Record(record) => {
                let id = record.id;
                let selector = format!("network-record-{id}");
                base.id(SharedString::from(format!("net-{id}")))
                    .debug_selector(move || selector.clone())
                    .cursor_pointer()
                    .bg(if self.selected == Some(id) { t.bg_active } else { t.bg_panel })
                    .text_color(if state_of(record) == NetState::Failed {
                        t.err
                    } else if is_slow(record) {
                        t.warn
                    } else {
                        t.fg
                    })
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.selected = Some(id);
                        cx.notify();
                    }))
                    .child(format!(
                        "{} {} · {} · {}",
                        record.method,
                        record.url,
                        status_text(record),
                        ms_text(record.ms)
                    ))
                    .into_any_element()
            }
        }
    }
    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let record = self.log.get(self.selected?)?;
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut view = div()
            .id("network-details")
            .debug_selector(|| "network-details".into())
            .w(t.px(300.))
            .min_w_0()
            .flex_none()
            .overflow_y_scroll()
            .p(t.px(9.))
            .flex()
            .flex_col()
            .gap(t.px(5.))
            .text_size(t.px(11.));
        view = view
            .child(format!("{} {}", record.method, record.url))
            .child(format!("{} · {} · {}", status_text(record), ms_text(record.ms), record.lib))
            .child(func_label(record.func.as_deref()));
        if let Some(func) = &record.func {
            let func = func.clone();
            view = view.child(
                div()
                    .id("network-open-source")
                    .debug_selector(|| "network-open-source".into())
                    .cursor_pointer()
                    .text_color(t.link)
                    .child("Open calling function")
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(NetworkEvent::OpenFunc(func.clone())))),
            );
        }
        if let Some(error) = &record.error {
            view = view.child(div().text_color(t.err).child(error.clone()));
        }
        for (title, headers, body) in
            [("Request", &record.req_headers, &record.req_body), ("Response", &record.resp_headers, &record.resp_body)]
        {
            view = view.child(div().text_color(t.heat_2).child(title));
            for (name, value) in headers {
                view = view.child(format!("{name}: {value}"));
            }
            if let Some(body) = body {
                for line in pretty_body(body).lines() {
                    view = view.child(line.to_owned());
                }
            }
        }
        if let Some(bytes) = record.size {
            view = view.child(format!("{bytes} bytes received"));
        }
        Some(view.into_any_element())
    }
}
impl Render for NetworkPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let lanes = self.timeline.clone();
        let end = self.timeline_end;
        let theme = t.clone();
        let selected = self.selected;
        let timeline = canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| {
                let count = lanes.iter().map(|(l, _)| l.lane + 1).max().unwrap_or(1);
                let h = (f32::from(bounds.size.height) / count as f32).min(14.);
                for (lane, state) in lanes.iter() {
                    let x = (lane.start / end).clamp(0., 1.) as f32 * f32::from(bounds.size.width);
                    let w = ((lane.end - lane.start) / end).clamp(0., 1.) as f32 * f32::from(bounds.size.width);
                    let color = if selected == Some(lane.id) {
                        theme.heat_2
                    } else {
                        match state {
                            NetState::Failed => theme.err,
                            NetState::Pending => theme.warn,
                            NetState::Ok => theme.graph_out,
                        }
                    };
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.origin.x + px(x), bounds.origin.y + px(lane.lane as f32 * h)),
                            size(px(w.max(2.)), px((h * 0.85).max(0.1))),
                        ),
                        color,
                    ));
                }
            },
        )
        .w_full()
        .h(t.px(90.))
        .flex_none();
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .text_color(t.fg)
            .bg(t.bg_panel)
            .child(div().p(t.px(6.)).child(Input::new(&self.filter).w_full()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(t.px(10.))
                    .px(t.px(7.))
                    .text_size(t.px(11.))
                    .child(
                        div()
                            .id("network-only")
                            .debug_selector(|| "network-only".into())
                            .cursor_pointer()
                            .child(format!("Show: {:?}", self.only))
                            .on_click(cx.listener(|p, _, _, cx| {
                                p.only = match p.only {
                                    Only::All => Only::Failed,
                                    Only::Failed => Only::Slow,
                                    Only::Slow => Only::All,
                                };
                                p.refresh(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("network-group")
                            .debug_selector(|| "network-group".into())
                            .cursor_pointer()
                            .child(format!("Group: {:?}", self.group))
                            .on_click(cx.listener(|p, _, _, cx| {
                                p.group = match p.group {
                                    GroupBy::None => GroupBy::Host,
                                    GroupBy::Host => GroupBy::Func,
                                    GroupBy::Func => GroupBy::None,
                                };
                                p.refresh(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("network-compare")
                            .debug_selector(|| "network-compare".into())
                            .cursor_pointer()
                            .child(if self.comparing { "Show this run" } else { "Compare last run" })
                            .on_click(cx.listener(|p, _, _, cx| {
                                p.comparing = !p.comparing;
                                p.selected = None;
                                p.refresh(cx);
                            })),
                    )
                    .child(div().id("network-clear").cursor_pointer().child("Clear").on_click(cx.listener(
                        |p, _, _, cx| {
                            p.log.clear();
                            p.selected = None;
                            p.refresh(cx);
                        },
                    ))),
            )
            .child(timeline)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "network-history",
                            self.rows.len(),
                            cx.processor(|p, range: std::ops::Range<usize>, _, cx| {
                                range.map(|i| p.row(i, cx)).collect()
                            }),
                        )
                        .flex_1()
                        .min_w_0()
                        .min_h_0(),
                    )
                    .children(self.details(cx)),
            )
            .child(div().p(t.px(6.)).text_size(t.px(10.)).text_color(t.fg_muted).child(self.summary.clone()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    #[gpui_kit::test]
    fn requests_can_be_selected_filtered_and_compared_with_the_previous_run(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(crate::theme::ForgeTheme(std::sync::Arc::new(crate::theme::Theme::molten())));
        });
        let mut panel = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let p = cx.new(|cx| NetworkPanel::new(window, cx));
            panel = Some(p.clone());
            gpui_kit::component::Root::new(p, window, cx)
        });
        let panel = panel.unwrap();
        panel.update(cx, |p, cx| {
            p.begin(1, cx);
            p.upsert(
                std::sync::Arc::new(forge_core::network::NetRecord {
                    id: 1,
                    method: "GET".into(),
                    url: "https://api.test/users".into(),
                    host: "api.test".into(),
                    ms: Some(30.),
                    status: Some(200),
                    ..Default::default()
                }),
                cx,
            );
            p.upsert(
                std::sync::Arc::new(forge_core::network::NetRecord {
                    id: 2,
                    method: "POST".into(),
                    url: "https://api.test/orders".into(),
                    host: "api.test".into(),
                    ms: Some(1300.),
                    status: Some(500),
                    resp_body: Some("{\"message\":\"failed\"}".into()),
                    ..Default::default()
                }),
                cx,
            );
            p.refresh(cx);
        });
        cx.run_until_parked();
        let row = cx.debug_bounds("network-record-2").expect("Request history row");
        cx.simulate_click(row.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        panel.read_with(cx, |p, _| assert_eq!(p.selected, Some(2)));
        assert!(cx.debug_bounds("network-details").is_some());
        let filter = cx.debug_bounds("network-only").unwrap();
        cx.simulate_click(filter.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        panel.read_with(cx, |p, _| {
            assert_eq!(p.only, forge_core::network::Only::Failed);
            assert_eq!(p.filtered.len(), 1);
        });
        panel.update(cx, |p, cx| {
            p.begin(2, cx);
            p.upsert(
                std::sync::Arc::new(forge_core::network::NetRecord {
                    id: 1,
                    method: "POST".into(),
                    url: "https://api.test/orders".into(),
                    ms: Some(10.),
                    status: Some(200),
                    ..Default::default()
                }),
                cx,
            );
            p.refresh(cx);
        });
        cx.run_until_parked();
        let compare = cx.debug_bounds("network-compare").unwrap();
        cx.simulate_click(compare.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        panel.read_with(cx, |p, _| {
            assert!(p.comparing);
            assert!(!p.log.previous().is_empty());
        });
    }
}
