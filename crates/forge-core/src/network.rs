//! Runtime network records, filtering, timeline and run comparisons,
//! replacing `shared/network.ts` and the pure `renderer/networkView.ts` helpers.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetKind {
    #[default]
    Http,
    Socket,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NetRecord {
    pub id: i64,
    pub kind: NetKind,
    pub lib: String,
    pub method: String,
    pub url: String,
    pub host: String,
    pub status: Option<i64>,
    pub error: Option<String>,
    pub start: f64,
    pub ms: Option<f64>,
    pub func: Option<String>,
    pub req_headers: Vec<(String, String)>,
    pub resp_headers: Vec<(String, String)>,
    pub req_body: Option<String>,
    pub resp_body: Option<String>,
    pub size: Option<f64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetState {
    Pending,
    Ok,
    Failed,
}
pub const SLOW_MS: f64 = 1000.;
pub fn parse_net(raw: &serde_json::Value) -> Option<NetRecord> {
    if raw["t"] != "net" {
        return None;
    }
    let r = &raw["r"];
    let id = r["id"].as_i64()?;
    let kind = match r["kind"].as_str()? {
        "http" => NetKind::Http,
        "socket" => NetKind::Socket,
        _ => return None,
    };
    let number = |v: &serde_json::Value| v.as_f64().filter(|n| n.is_finite());
    let text = |key: &str| r[key].as_str().map(str::to_owned);
    let pairs = |key: &str| {
        r[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let p = p.as_array()?;
                if p.len() != 2 {
                    return None;
                }
                Some((p[0].as_str()?.into(), p[1].as_str()?.into()))
            })
            .collect()
    };
    Some(NetRecord {
        id,
        kind,
        lib: text("lib").unwrap_or_default(),
        method: r["method"].as_str()?.into(),
        url: r["url"].as_str()?.into(),
        host: text("host").unwrap_or_default(),
        status: r["status"].as_i64(),
        error: text("error"),
        start: number(&r["start"])?,
        ms: number(&r["ms"]),
        func: text("func"),
        req_headers: pairs("reqHeaders"),
        resp_headers: pairs("respHeaders"),
        req_body: text("reqBody"),
        resp_body: text("respBody"),
        size: number(&r["size"]),
    })
}
pub fn state_of(r: &NetRecord) -> NetState {
    if r.error.as_ref().is_some_and(|s| !s.is_empty()) || r.status.is_some_and(|s| s >= 400) {
        NetState::Failed
    } else if r.ms.is_none() {
        NetState::Pending
    } else {
        NetState::Ok
    }
}
pub fn is_slow(r: &NetRecord) -> bool {
    r.ms.is_some_and(|ms| ms >= SLOW_MS)
}
pub fn endpoint_of(r: &NetRecord) -> String {
    if r.kind == NetKind::Socket {
        return format!("{} {}", r.method, r.url);
    }
    static UUID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^[0-9a-f]{8}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{12}$")
            .expect("fixed UUID regex")
    });
    let path = r.url.split_once("://").map(|(_, p)| p).unwrap_or(&r.url).split(['?', '#']).next().unwrap_or("");
    let folded = path
        .split('/')
        .enumerate()
        .map(|(i, p)| {
            if i > 0 && !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                "{n}"
            } else if i > 0 && UUID.is_match(p) {
                "{id}"
            } else {
                p
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("{} {folded}", r.method)
}
pub fn func_label(func: Option<&str>) -> String {
    match func {
        None | Some("") => "(outside your code)".into(),
        Some(s) if s.starts_with("module:") => format!("{} (top level)", &s[7..]),
        Some(s) => s.replacen(':', ".", 1),
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Only {
    #[default]
    All,
    Failed,
    Slow,
}
#[derive(Clone, Debug, Default)]
pub struct NetFilter {
    pub text: String,
    pub only: Only,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GroupBy {
    #[default]
    None,
    Host,
    Func,
}
pub fn filter_records(records: &[Arc<NetRecord>], f: &NetFilter) -> Vec<Arc<NetRecord>> {
    let q = f.text.trim().to_lowercase();
    records
        .iter()
        .filter(|r| {
            if f.only == Only::Failed && state_of(r) != NetState::Failed || f.only == Only::Slow && !is_slow(r) {
                return false;
            }
            q.is_empty()
                || [
                    r.method.as_str(),
                    &r.url,
                    &r.status.map(|s| s.to_string()).unwrap_or_default(),
                    r.error.as_deref().unwrap_or(""),
                    &func_label(r.func.as_deref()),
                    &r.lib,
                ]
                .iter()
                .any(|s| s.to_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}
pub struct NetGroup {
    pub key: String,
    pub label: String,
    pub records: Vec<Arc<NetRecord>>,
    pub failed: usize,
    pub slowest: f64,
}
pub fn group_records(records: &[Arc<NetRecord>], by: GroupBy) -> Vec<NetGroup> {
    let mut groups: BTreeMap<String, Vec<Arc<NetRecord>>> = BTreeMap::new();
    if by == GroupBy::None {
        groups.insert(String::new(), records.to_vec());
    } else {
        for r in records {
            groups
                .entry(if by == GroupBy::Host { r.host.clone() } else { r.func.clone().unwrap_or_default() })
                .or_default()
                .push(r.clone());
        }
    }
    let mut groups = groups
        .into_iter()
        .map(|(key, records)| NetGroup {
            label: match by {
                GroupBy::None => String::new(),
                GroupBy::Host => {
                    if key.is_empty() {
                        "(unknown host)".into()
                    } else {
                        key.clone()
                    }
                }
                GroupBy::Func => func_label(Some(&key)),
            },
            key,
            failed: records.iter().filter(|r| state_of(r) == NetState::Failed).count(),
            slowest: records.iter().filter_map(|r| r.ms).fold(0., f64::max),
            records,
        })
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| a.key.is_empty().cmp(&b.key.is_empty()).then(a.label.cmp(&b.label)));
    groups
}
#[derive(Clone, Debug)]
pub struct Lane {
    pub id: i64,
    pub start: f64,
    pub end: f64,
    pub lane: usize,
}
pub fn timeline_lanes(records: &[Arc<NetRecord>], now: f64) -> Vec<Lane> {
    let mut records = records.to_vec();
    records.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.id.cmp(&b.id)));
    let mut ends = Vec::new();
    records
        .iter()
        .map(|r| {
            let end = r.ms.map(|ms| (r.start + ms).clamp(0., f64::MAX)).unwrap_or(now.max(r.start));
            let lane = ends.iter().position(|e| *e <= r.start).unwrap_or(ends.len());
            if lane == ends.len() {
                ends.push(end)
            } else {
                ends[lane] = end
            }
            Lane { id: r.id, start: r.start, end, lane }
        })
        .collect()
}
#[derive(Clone, Debug, Default)]
pub struct RunSide {
    pub count: usize,
    pub avg_ms: f64,
    pub failed: usize,
}
pub struct CompareRow {
    pub endpoint: String,
    pub before: RunSide,
    pub after: RunSide,
    pub change: &'static str,
}
pub fn compare_runs(before: &[Arc<NetRecord>], after: &[Arc<NetRecord>]) -> Vec<CompareRow> {
    fn sides(records: &[Arc<NetRecord>]) -> HashMap<String, RunSide> {
        let mut map: HashMap<String, (RunSide, f64, usize)> = HashMap::new();
        for r in records {
            let (side, total, timed) = map.entry(endpoint_of(r)).or_default();
            side.count += 1;
            side.failed += usize::from(state_of(r) == NetState::Failed);
            if let Some(ms) = r.ms {
                *total += ms;
                *timed += 1;
            }
        }
        map.into_iter()
            .map(|(k, (mut side, total, timed))| {
                side.avg_ms = if timed > 0 { (total / timed as f64).round() } else { 0. };
                (k, side)
            })
            .collect()
    }
    let b = sides(before);
    let a = sides(after);
    let mut endpoints = b.keys().chain(a.keys()).cloned().collect::<Vec<_>>();
    endpoints.sort();
    endpoints.dedup();
    let mut rows = endpoints
        .into_iter()
        .map(|endpoint| {
            let x = b.get(&endpoint).cloned().unwrap_or_default();
            let y = a.get(&endpoint).cloned().unwrap_or_default();
            let rate = |s: &RunSide| if s.count > 0 { s.failed as f64 / s.count as f64 } else { 0. };
            let change = if x.count == 0 {
                "new"
            } else if y.count == 0 {
                "gone"
            } else if rate(&y) > rate(&x) {
                "failing"
            } else if rate(&y) < rate(&x) {
                "fixed"
            } else if y.avg_ms - x.avg_ms >= 50. && y.avg_ms >= x.avg_ms * 1.5 {
                "slower"
            } else if x.avg_ms - y.avg_ms >= 50. && x.avg_ms >= y.avg_ms * 1.5 {
                "faster"
            } else if y.count > x.count {
                "more"
            } else if y.count < x.count {
                "fewer"
            } else {
                "same"
            };
            CompareRow { endpoint, before: x, after: y, change }
        })
        .collect::<Vec<_>>();
    let rank = |s: &str| {
        ["failing", "new", "gone", "slower", "more", "fewer", "faster", "fixed", "same"]
            .iter()
            .position(|v| *v == s)
            .unwrap_or(9)
    };
    rows.sort_by(|a, b| rank(a.change).cmp(&rank(b.change)).then(a.endpoint.cmp(&b.endpoint)));
    rows
}
/// Matches the tracer cap; Arc records make previous-run retention and view
/// projections cheap without copying request/response bodies.
#[derive(Default)]
pub struct NetLog {
    now: BTreeMap<i64, Arc<NetRecord>>,
    before: Vec<Arc<NetRecord>>,
    pub run_id: u64,
    pub dropped: usize,
}
impl NetLog {
    pub fn begin(&mut self, id: u64) {
        if !self.now.is_empty() {
            self.before = self.current()
        }
        self.now.clear();
        self.run_id = id;
        self.dropped = 0;
    }
    pub fn clear(&mut self) {
        self.now.clear();
        self.before.clear();
        self.dropped = 0;
    }
    pub fn upsert(&mut self, r: Arc<NetRecord>) {
        if self.now.len() >= 2000 && !self.now.contains_key(&r.id) {
            self.dropped += 1;
            return;
        }
        self.now.insert(r.id, r);
    }
    pub fn get(&self, id: i64) -> Option<&Arc<NetRecord>> {
        self.now.get(&id)
    }
    pub fn current(&self) -> Vec<Arc<NetRecord>> {
        self.now.values().cloned().collect()
    }
    pub fn previous(&self) -> &[Arc<NetRecord>] {
        &self.before
    }
}
pub fn status_text(r: &NetRecord) -> String {
    if let Some(error) = r.error.as_deref().filter(|s| !s.is_empty()) {
        error.split(':').next().unwrap_or(error).into()
    } else if let Some(status) = r.status {
        status.to_string()
    } else if r.kind == NetKind::Socket && r.ms.is_some() {
        "connected".into()
    } else {
        "…".into()
    }
}
pub fn ms_text(ms: Option<f64>) -> String {
    match ms {
        None => String::new(),
        Some(ms) if ms >= 10000. => format!("{:.0} s", (ms / 1000.).round()),
        Some(ms) if ms >= 1000. => format!("{:.1} s", (ms / 100.).round() / 10.),
        Some(ms) => format!("{:.0} ms", ms.round()),
    }
}
pub fn pretty_body(body: &str) -> String {
    if body.trim().starts_with(['{', '[']) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            return serde_json::to_string_pretty(&value).unwrap_or_else(|_| body.into());
        }
    }
    body.into()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn rec(over: serde_json::Value) -> Arc<NetRecord> {
        let mut raw = json!({"id":1,"kind":"http","lib":"requests","method":"GET","url":"https://api.example.com/users","host":"api.example.com","status":200,"error":null,"start":0,"ms":50,"func":"app.api:fetch","reqHeaders":[],"respHeaders":[],"reqBody":null,"respBody":null,"size":null});
        for (k, v) in over.as_object().unwrap() {
            raw[k] = v.clone()
        }
        Arc::new(parse_net(&json!({"t":"net","r":raw})).unwrap())
    }
    #[test]
    fn reads_tracer_messages_rejects_junk_and_drops_bad_header_pairs() {
        let r = rec(json!({"id":7}));
        assert_eq!(parse_net(&json!({"t":"net","r":r})).unwrap(), *r);
        for raw in [
            json!(null),
            json!({"t":"tick"}),
            json!({"t":"net","r":{"id":"x"}}),
            json!({"t":"net","r":{"id":1,"kind":"ftp"}}),
        ] {
            assert!(parse_net(&raw).is_none())
        }
        assert_eq!(rec(json!({"reqHeaders":[["a","b"],["bad"],3]})).req_headers, vec![("a".into(), "b".into())]);
    }
    #[test]
    fn pending_errors_http_status_socket_slowness_and_endpoint_folding() {
        assert_eq!(state_of(&rec(json!({"ms":null,"status":null}))), NetState::Pending);
        for status in [200, 304] {
            assert_eq!(state_of(&rec(json!({"status":status}))), NetState::Ok)
        }
        assert_eq!(state_of(&rec(json!({"status":404}))), NetState::Failed);
        assert_eq!(state_of(&rec(json!({"error":"Refused","status":null}))), NetState::Failed);
        assert_eq!(state_of(&rec(json!({"kind":"socket","method":"CONNECT","status":null}))), NetState::Ok);
        assert!(!is_slow(&rec(json!({"ms":999}))));
        assert!(is_slow(&rec(json!({"ms":1000}))));
        assert!(!is_slow(&rec(json!({"ms":null}))));
        for (over, expected) in [
            (json!({"url":"https://api.example.com/users/42/posts?page=2"}), "GET api.example.com/users/{n}/posts"),
            (
                json!({"url":"https://api.example.com/items/3f2b1c9e-0d6a-4c3f-9a1b-2b3c4d5e6f70"}),
                "GET api.example.com/items/{id}",
            ),
            (json!({"kind":"socket","method":"CONNECT","url":"db.local:5432"}), "CONNECT db.local:5432"),
        ] {
            assert_eq!(endpoint_of(&rec(over)), expected)
        }
    }
    #[test]
    fn filters_text_status_function_and_failed_or_slow_and_groups_summaries() {
        let records = vec![
            rec(json!({"id":1,"ms":40})),
            rec(json!({"id":2,"url":"https://api.example.com/orders","status":500,"ms":80,"func":"app.api:orders"})),
            rec(
                json!({"id":3,"url":"https://hooks.slack.com/x","host":"hooks.slack.com","method":"POST","ms":1500,"func":null}),
            ),
        ];
        for (q, only, ids) in [
            ("slack", Only::All, vec![3]),
            ("post", Only::All, vec![3]),
            ("500", Only::All, vec![2]),
            ("orders", Only::All, vec![2]),
            ("", Only::Failed, vec![2]),
            ("", Only::Slow, vec![3]),
        ] {
            assert_eq!(
                filter_records(&records, &NetFilter { text: q.into(), only }).iter().map(|r| r.id).collect::<Vec<_>>(),
                ids
            )
        }
        let groups = group_records(&records, GroupBy::Host);
        assert_eq!(
            groups.iter().map(|g| format!("{}:{}:{}", g.label, g.records.len(), g.failed)).collect::<Vec<_>>(),
            vec!["api.example.com:2:1", "hooks.slack.com:1:0"]
        );
        assert_eq!(
            group_records(&records, GroupBy::Func).iter().map(|g| g.label.as_str()).collect::<Vec<_>>(),
            vec!["app.api.fetch", "app.api.orders", "(outside your code)"]
        );
        let g = group_records(&records, GroupBy::None);
        assert_eq!(g[0].slowest, 1500.);
        assert_eq!(g[0].failed, 1);
    }
    #[test]
    fn overlapping_requests_get_separate_lanes_and_pending_runs_to_now() {
        let records = vec![
            rec(json!({"id":1,"start":0,"ms":100})),
            rec(json!({"id":2,"start":50,"ms":100})),
            rec(json!({"id":3,"start":120,"ms":10})),
            rec(json!({"id":4,"start":130,"ms":null})),
        ];
        let lanes = timeline_lanes(&records, 200.);
        assert_eq!(lanes.iter().map(|l| (l.id, l.lane)).collect::<Vec<_>>(), vec![(1, 0), (2, 1), (3, 0), (4, 0)]);
        assert_eq!(lanes[3].end, 200.);
    }
    #[test]
    fn compares_new_gone_slower_faster_and_failure_rates() {
        let before = vec![
            rec(json!({"url":"https://a.com/users/1","ms":100})),
            rec(json!({"url":"https://a.com/users/2","ms":100})),
            rec(json!({"url":"https://a.com/old","ms":10})),
            rec(json!({"url":"https://a.com/fast","ms":900})),
            rec(json!({"url":"https://a.com/flaky","ms":10})),
        ];
        let after = vec![
            rec(json!({"url":"https://a.com/users/3","ms":400})),
            rec(json!({"url":"https://a.com/new","ms":10})),
            rec(json!({"url":"https://a.com/fast","ms":100})),
            rec(json!({"url":"https://a.com/flaky","ms":10,"status":503})),
        ];
        let rows = compare_runs(&before, &after);
        for (end, change) in
            [("users/{n}", "slower"), ("new", "new"), ("old", "gone"), ("fast", "faster"), ("flaky", "failing")]
        {
            assert_eq!(rows.iter().find(|r| r.endpoint == format!("GET a.com/{end}")).unwrap().change, change)
        }
        assert_eq!(rows[0].change, "failing");
    }
    #[test]
    fn log_updates_by_id_keeps_previous_nonempty_run_and_clear_forgets_both() {
        let mut log = NetLog::default();
        log.begin(1);
        log.upsert(rec(json!({"status":null,"ms":null})));
        log.upsert(rec(json!({"status":200,"ms":30})));
        assert_eq!(log.current().len(), 1);
        log.begin(2);
        log.upsert(rec(json!({"url":"https://x.com/"})));
        assert_eq!(log.previous()[0].url, "https://api.example.com/users");
        log.begin(3);
        log.begin(4);
        assert_eq!(log.previous()[0].url, "https://x.com/");
        log.clear();
        assert!(log.current().is_empty());
        assert!(log.previous().is_empty());
    }
    #[test]
    fn panel_text_formats_status_times_and_json_bodies() {
        assert_eq!(status_text(&rec(json!({"status":404}))), "404");
        assert_eq!(
            status_text(&rec(json!({"error":"ConnectionRefusedError: refused","status":null}))),
            "ConnectionRefusedError"
        );
        assert_eq!(status_text(&rec(json!({"status":null,"ms":null}))), "…");
        assert_eq!(status_text(&rec(json!({"kind":"socket","status":null}))), "connected");
        assert_eq!(ms_text(None), "");
        for (ms, text) in [(12.4, "12 ms"), (1534., "1.5 s"), (23000., "23 s")] {
            assert_eq!(ms_text(Some(ms)), text)
        }
        assert_eq!(pretty_body("{\"a\":1}"), "{\n  \"a\": 1\n}");
        for text in ["{\"a\":1, \"b\"", "name=ann"] {
            assert_eq!(pretty_body(text), text)
        }
    }
}
