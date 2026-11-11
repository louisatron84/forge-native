//! Visual profiler data and paths, replacing `src/shared/profile.ts`.
use crate::graph_view::GraphMode;
use std::collections::{BTreeMap, HashSet};
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FnTime {
    pub calls: i64,
    pub total_ms: f64,
    pub self_ms: f64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunProfile {
    pub fns: BTreeMap<String, FnTime>,
    pub edges: BTreeMap<(String, String), f64>,
}
pub fn parse_profile(raw: &serde_json::Value) -> Option<RunProfile> {
    if raw.get("t")?.as_str()? != "prof" {
        return None;
    }
    let fns = raw.get("fns")?.as_array()?;
    let edges = raw.get("edges")?.as_array()?;
    let number = |v: &serde_json::Value| v.as_f64().filter(|n| n.is_finite() && *n >= 0.0);
    let mut profile = RunProfile::default();
    for row in fns {
        let Some(row) = row.as_array() else {
            continue;
        };
        if row.len() < 4 {
            continue;
        }
        if let (Some(id), Some(calls), Some(total_ms), Some(self_ms)) =
            (row[0].as_str(), row[1].as_i64(), number(&row[2]), number(&row[3]))
        {
            profile.fns.insert(id.into(), FnTime { calls, total_ms, self_ms });
        }
    }
    for row in edges {
        let Some(row) = row.as_array() else {
            continue;
        };
        if row.len() < 3 {
            continue;
        }
        if let (Some(from), Some(to), Some(ms)) = (row[0].as_str(), row[1].as_str(), number(&row[2])) {
            profile.edges.insert((from.into(), to.into()), ms);
        }
    }
    Some(profile)
}
pub fn view_times(profile: &RunProfile, mode: GraphMode) -> BTreeMap<String, FnTime> {
    if mode != GraphMode::Modules {
        return profile.fns.clone();
    }
    let mut out: BTreeMap<String, FnTime> = BTreeMap::new();
    for (id, time) in &profile.fns {
        let entry = out.entry(module_of(id).into()).or_default();
        entry.calls += time.calls;
        entry.self_ms += time.self_ms;
        entry.total_ms = entry.self_ms;
    }
    out
}
pub fn heat_level(ms: f64, max: f64) -> u8 {
    if ms < 1.0 || max <= 0.0 {
        return 0;
    }
    let share = ms / max;
    if share >= 0.5 {
        3
    } else if share >= 0.2 {
        2
    } else if share >= 0.05 {
        1
    } else {
        0
    }
}
pub fn time_path(profile: &RunProfile, id: &str) -> Vec<String> {
    let mut callers: BTreeMap<&str, Vec<(&str, f64)>> = BTreeMap::new();
    for ((from, to), ms) in &profile.edges {
        callers.entry(to).or_default().push((from, *ms));
    }
    let mut path = vec![id.to_owned()];
    let mut seen = HashSet::from([id.to_owned()]);
    let mut at = id;
    loop {
        let best = callers
            .get(at)
            .and_then(|list| list.iter().filter(|(from, _)| !seen.contains(*from)).max_by(|a, b| a.1.total_cmp(&b.1)));
        let Some((from, _)) = best else {
            break;
        };
        at = from;
        seen.insert(at.into());
        path.push(at.into());
    }
    path.reverse();
    path
}
pub fn slowest(profile: &RunProfile, count: usize) -> Vec<(String, FnTime)> {
    let mut out: Vec<_> = profile.fns.iter().map(|(id, time)| (id.clone(), *time)).collect();
    out.sort_by(|a, b| b.1.self_ms.total_cmp(&a.1.self_ms));
    out.truncate(count);
    out
}
pub fn format_ms(ms: f64) -> String {
    if ms < 0.05 {
        "<0.1 ms".into()
    } else if ms < 10.0 {
        format!("{ms:.1} ms")
    } else if ms < 1000.0 {
        format!("{} ms", ms.round())
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}
pub fn module_of(id: &str) -> &str {
    id.strip_prefix("module:").unwrap_or_else(|| id.split(':').next().unwrap_or(id))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn msg() -> serde_json::Value {
        json!({"t":"prof","fns":[["module:app",1,100,5],["app:main",1,95,5],["app:load",2,80,10],["util.text:parse",40,70,70],["bad","x",1,1]],"edges":[["module:app","app:main",95],["app:main","app:load",80],["app:main","util.text:parse",10],["app:load","util.text:parse",60],["app:load"]]})
    }
    #[test]
    fn reads_tracer_messages_and_drops_malformed_rows() {
        let p = parse_profile(&msg()).unwrap();
        assert_eq!(p.fns.len(), 4);
        assert_eq!(p.fns["util.text:parse"], FnTime { calls: 40, total_ms: 70.0, self_ms: 70.0 });
        assert_eq!(p.edges.len(), 4);
        assert!(parse_profile(&json!({"t":"tick","at":null,"calls":[]})).is_none());
        assert!(parse_profile(&json!(null)).is_none());
    }
    #[test]
    fn call_times_are_per_function_module_times_are_own_time() {
        let p = parse_profile(&msg()).unwrap();
        assert_eq!(view_times(&p, GraphMode::Calls)["app:load"], FnTime { calls: 2, total_ms: 80.0, self_ms: 10.0 });
        assert_eq!(view_times(&p, GraphMode::Modules)["app"], FnTime { calls: 4, total_ms: 20.0, self_ms: 20.0 });
    }
    #[test]
    fn heats_functions_that_take_most_time() {
        for (ms, max, heat) in [(60.0, 100.0, 3), (30.0, 100.0, 2), (6.0, 100.0, 1), (2.0, 100.0, 0), (0.5, 0.6, 0)] {
            assert_eq!(heat_level(ms, max), heat);
        }
    }
    #[test]
    fn explains_heaviest_callers_from_program_start() {
        let p = parse_profile(&msg()).unwrap();
        assert_eq!(time_path(&p, "util.text:parse"), ["module:app", "app:main", "app:load", "util.text:parse"]);
        assert_eq!(time_path(&p, "module:app"), ["module:app"]);
    }
    #[test]
    fn stops_at_a_loop_of_callers() {
        let p = parse_profile(
            &json!({"t":"prof","fns":[["m:a",1,5,1],["m:b",1,5,1]],"edges":[["m:a","m:b",5],["m:b","m:a",4]]}),
        )
        .unwrap();
        assert_eq!(time_path(&p, "m:b"), ["m:a", "m:b"]);
    }
    #[test]
    fn lists_slowest_functions_by_own_time() {
        let p = parse_profile(&msg()).unwrap();
        assert_eq!(
            slowest(&p, 2).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["util.text:parse", "app:load"]
        );
    }
    #[test]
    fn formats_times_for_people() {
        for (ms, expected) in [(0.04, "<0.1 ms"), (0.42, "0.4 ms"), (12.3, "12 ms"), (1534.0, "1.5 s")] {
            assert_eq!(format_ms(ms), expected);
        }
    }
}
