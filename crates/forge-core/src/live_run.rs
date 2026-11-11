//! Live execution and network projection, replacing `graph/liveRun.ts`.
use crate::{
    graph_view::{GraphMode, GraphViewData},
    profile::module_of,
};
use std::collections::{HashMap, HashSet};
pub const MAX_SPARKS: usize = 12;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceTick {
    pub at: Option<String>,
    pub calls: Vec<(String, String, u64)>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TraceMessage {
    Tick(TraceTick),
    End,
}
#[derive(Clone, Debug, Default)]
pub struct RunView {
    mode: GraphMode,
    nodes: HashSet<String>,
    edges: HashSet<(String, String)>,
    events: Vec<String>,
    boxes: HashMap<String, Vec<String>>,
    net_boxes: HashMap<(String, String), String>,
}
impl RunView {
    pub fn new(mode: GraphMode, data: &GraphViewData) -> Self {
        Self {
            mode,
            nodes: data.nodes.iter().map(|n| n.id.clone()).collect(),
            edges: data.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect(),
            events: data.nodes.iter().filter(|n| n.id.starts_with("event:")).map(|n| n.id.clone()).collect(),
            boxes: HashMap::new(),
            net_boxes: HashMap::new(),
        }
    }
    pub fn attach_boxes(&mut self, boxes: &[crate::boxes::ApiBox]) {
        self.boxes.clear();
        self.net_boxes.clear();
        for api in boxes {
            if let Some(attach) = &api.attach {
                if let Some(host) = crate::project_graph::host_of(Some(&api.base_url)) {
                    self.net_boxes.entry((attach.module.clone(), host)).or_insert_with(|| format!("box:{}", api.id));
                }
                for name in crate::box_code::request_functions(api).values() {
                    self.boxes.entry(format!("{}:{name}", attach.module)).or_default().push(format!("box:{}", api.id));
                }
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct NetSpark {
    pub from: String,
    pub to: Option<String>,
    pub ok: bool,
}
pub fn plan_net_spark(record: &crate::network::NetRecord, view: &RunView) -> Option<NetSpark> {
    if record.kind != crate::network::NetKind::Http {
        return None;
    }
    let func = record.func.as_deref()?;
    let module = module_of(func);
    let from = if view.mode == GraphMode::Modules { module } else { func };
    if !view.nodes.contains(from) {
        return None;
    }
    let host = record.host.to_lowercase();
    let to = view.net_boxes.get(&(module.to_owned(), host.clone())).cloned().or_else(|| {
        let node = format!("host:{host}");
        view.nodes.contains(&node).then_some(node)
    });
    Some(NetSpark { from: from.into(), to, ok: crate::network::state_of(record) != crate::network::NetState::Failed })
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunFrame {
    pub glow: Option<String>,
    pub sparks: Vec<(String, String)>,
    pub boxes: HashSet<String>,
}
pub fn parse_trace(raw: &serde_json::Value) -> Option<TraceMessage> {
    let kind = raw.get("t")?.as_str()?;
    if kind == "end" {
        return Some(TraceMessage::End);
    }
    if kind != "tick" {
        return None;
    }
    let at = raw.get("at")?;
    let at = if at.is_null() { None } else { Some(at.as_str()?.into()) };
    let rows = raw.get("calls")?.as_array()?;
    let calls = rows
        .iter()
        .filter_map(|row| {
            let row = row.as_array()?;
            if row.len() != 3 {
                return None;
            }
            Some((row[0].as_str()?.into(), row[1].as_str()?.into(), row[2].as_u64().filter(|n| *n > 0)?))
        })
        .collect();
    Some(TraceMessage::Tick(TraceTick { at, calls }))
}
pub fn plan_frame(tick: &TraceTick, view: &RunView) -> RunFrame {
    let to_view = |id: &str| if view.mode == GraphMode::Modules { module_of(id).to_owned() } else { id.to_owned() };
    let glow = tick.at.as_deref().map(to_view).filter(|id| view.nodes.contains(id));
    let mut counts: Vec<((String, String), u64)> = Vec::new();
    let mut indices = HashMap::new();
    let mut count = |from: &str, to: &str, n: u64| {
        let pair = (from.to_owned(), to.to_owned());
        let i = *indices.entry(pair.clone()).or_insert_with(|| {
            counts.push((pair, 0));
            counts.len() - 1
        });
        counts[i].1 = counts[i].1.saturating_add(n);
    };
    for (a, b, n) in &tick.calls {
        let from = to_view(a);
        let to = to_view(b);
        if from == to {
            continue;
        }
        if view.mode == GraphMode::Events && !view.edges.contains(&(from.clone(), to.clone())) {
            let through: Vec<_> =
                view.events.iter().filter(|e| view.edges.contains(&((*e).clone(), to.clone()))).collect();
            if let Some(event) = through.iter().find(|e| view.edges.contains(&(from.clone(), e.as_str().to_owned()))) {
                count(&from, event, *n);
                count(event, &to, *n);
            } else if let Some(event) = through.first() {
                count(event, &to, *n);
            }
            continue;
        }
        if view.edges.contains(&(from.clone(), to.clone()))
            || (view.mode == GraphMode::Modules && view.edges.contains(&(to.clone(), from.clone())))
        {
            count(&from, &to, *n);
        }
    }
    counts.sort_by_key(|a| std::cmp::Reverse(a.1));
    counts.truncate(MAX_SPARKS);
    let boxes = tick
        .at
        .iter()
        .chain(tick.calls.iter().map(|(_, to, _)| to))
        .filter_map(|id| view.boxes.get(id))
        .flatten()
        .cloned()
        .collect();
    RunFrame { glow, sparks: counts.into_iter().map(|(pair, _)| pair).collect(), boxes }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn network_sparks_follow_shown_hosts_boxes_and_callers_and_ignore_sockets() {
        let data = GraphViewData {
            nodes: ["app.api", "host:api.github.com"]
                .iter()
                .map(|id| crate::graph_view::ViewNode { id: (*id).into(), ..Default::default() })
                .collect(),
            ..Default::default()
        };
        let mut view = RunView::new(GraphMode::Modules, &data);
        let mut r = crate::network::NetRecord {
            func: Some("app.api:fetch".into()),
            host: "api.github.com".into(),
            ..Default::default()
        };
        let spark = plan_net_spark(&r, &view).unwrap();
        assert_eq!(spark.from, "app.api");
        assert_eq!(spark.to.as_deref(), Some("host:api.github.com"));
        assert!(spark.ok);
        r.status = Some(500);
        assert!(!plan_net_spark(&r, &view).unwrap().ok);
        let mut api = crate::boxes::new_api_box(&[], "GitHub");
        api.base_url = "https://api.github.com".into();
        api.attach = Some(crate::boxes::BoxAttach { module: "app.api".into() });
        view.attach_boxes(&[api.clone()]);
        assert_eq!(plan_net_spark(&r, &view).unwrap().to, Some(format!("box:{}", api.id)));
        r.kind = crate::network::NetKind::Socket;
        assert!(plan_net_spark(&r, &view).is_none());
        r.kind = crate::network::NetKind::Http;
        r.func = None;
        assert!(plan_net_spark(&r, &view).is_none());
        r.func = Some("other:f".into());
        assert!(plan_net_spark(&r, &view).is_none());
        r.func = Some("app.api:fetch".into());
        let calls = RunView::new(
            GraphMode::Calls,
            &GraphViewData {
                nodes: vec![crate::graph_view::ViewNode { id: "app.api:fetch".into(), ..Default::default() }],
                ..Default::default()
            },
        );
        assert!(plan_net_spark(&r, &calls).unwrap().to.is_none());
    }
    #[test]
    fn generated_request_activity_lights_only_its_attached_boxes() {
        let mut api = crate::boxes::new_api_box(&[], "GitHub");
        api.attach = Some(crate::boxes::BoxAttach { module: "app.api".into() });
        api.requests[0].name = "users".into();
        let mut v = RunView::default();
        v.attach_boxes(&[api.clone()]);
        let frame = plan_frame(&TraceTick { at: Some("app.api:github_users".into()), ..Default::default() }, &v);
        assert_eq!(frame.boxes, HashSet::from([format!("box:{}", api.id)]));
        assert!(plan_frame(&TraceTick { at: Some("other:github_users".into()), ..Default::default() }, &v)
            .boxes
            .is_empty());
        assert!(!plan_frame(
            &TraceTick { calls: vec![("main:run".into(), "app.api:github_users".into(), 1)], ..Default::default() },
            &v
        )
        .boxes
        .is_empty());
    }
    fn view(mode: GraphMode, pairs: &[(&str, &str)]) -> RunView {
        RunView {
            mode,
            nodes: HashSet::from([
                "module:main".into(),
                "main:start".into(),
                "pkg.b:work".into(),
                "main".into(),
                "pkg.b".into(),
            ]),
            edges: pairs.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect(),
            events: Vec::new(),
            boxes: HashMap::new(),
            net_boxes: HashMap::new(),
        }
    }
    #[test]
    fn ticks_end_and_junk() {
        assert!(matches!(
            parse_trace(&json!({"t":"tick","at":"a:f","calls":[["a:f","b:g",3]]})),
            Some(TraceMessage::Tick(_))
        ));
        assert_eq!(parse_trace(&json!({"t":"end"})), Some(TraceMessage::End));
        assert!(parse_trace(&json!({"t":"hello"})).is_none());
        assert!(parse_trace(&json!({"t":"tick","at":5,"calls":[]})).is_none());
        let Some(TraceMessage::Tick(tick)) =
            parse_trace(&json!({"t":"tick","at":"a:f","calls":[["a:f","b:g",-1],["x"],["a:f","c:h",2]]}))
        else {
            panic!("tick")
        };
        assert_eq!(tick.calls, vec![("a:f".into(), "c:h".into(), 2)]);
    }
    #[test]
    fn calls_view_glows_and_sparks_only_on_shown_edges() {
        let v = view(GraphMode::Calls, &[("main:start", "pkg.b:work"), ("module:main", "main:start")]);
        let tick = TraceTick {
            at: Some("pkg.b:work".into()),
            calls: vec![
                ("main:start".into(), "pkg.b:work".into(), 5),
                ("main:start".into(), "hidden:x".into(), 9),
                ("module:main".into(), "main:start".into(), 1),
            ],
        };
        let frame = plan_frame(&tick, &v);
        assert_eq!(frame.glow, Some("pkg.b:work".into()));
        assert_eq!(
            frame.sparks,
            vec![("main:start".into(), "pkg.b:work".into()), ("module:main".into(), "main:start".into())]
        );
    }
    #[test]
    fn module_view_sparks_callbacks_backwards_along_import_lines() {
        let v = view(GraphMode::Modules, &[("main", "pkg.b")]);
        let tick = TraceTick {
            at: Some("pkg.b:work".into()),
            calls: vec![
                ("main:start".into(), "pkg.b:work".into(), 5),
                ("pkg.b:work".into(), "main:start".into(), 1),
                ("main:start".into(), "module:main".into(), 2),
            ],
        };
        let f = plan_frame(&tick, &v);
        assert_eq!(f.glow, Some("pkg.b".into()));
        assert_eq!(f.sparks, vec![("main".into(), "pkg.b".into()), ("pkg.b".into(), "main".into())]);
    }
    #[test]
    fn at_most_twelve_sparks_busiest_first() {
        let pairs: Vec<(String, String, u64)> = (0..20).map(|i| ("a:f".into(), format!("b:g{i}"), i + 1)).collect();
        let v = RunView {
            mode: GraphMode::Calls,
            edges: pairs.iter().map(|(a, b, _)| (a.clone(), b.clone())).collect(),
            ..Default::default()
        };
        let f = plan_frame(&TraceTick { at: None, calls: pairs }, &v);
        assert_eq!(f.sparks.len(), 12);
        assert_eq!(f.sparks[0], ("a:f".into(), "b:g19".into()));
    }
}
