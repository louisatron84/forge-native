//! Bounded, memory-only inputs/outputs and line values from the Python tracer.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Summary {
    #[serde(rename = "type")]
    pub type_: String,
    pub kind: String,
    pub shape: Option<Vec<u64>>,
    pub columns: Option<Vec<(String, String)>>,
    pub index: Option<Vec<String>>,
    pub rows: Option<Vec<Vec<String>>>,
    pub dtype: Option<String>,
    pub name: Option<String>,
    pub len: Option<u64>,
    pub sample: Option<Vec<Value>>,
    pub repr: Option<String>,
}
#[derive(Clone, Debug)]
pub struct DataPoint {
    pub var: String,
    pub n: u64,
    pub summary: Arc<Summary>,
}
#[derive(Clone, Debug)]
pub struct IoCall {
    pub input: BTreeMap<String, Arc<Summary>>,
    pub output: Option<Arc<Summary>>,
}
#[derive(Clone, Debug)]
pub struct IoRecord {
    pub func: String,
    pub total: u64,
    pub calls: Vec<IoCall>,
}
#[derive(Clone, Debug)]
pub enum DataMessage {
    Data { func: String, line: u32, var: String, n: u64, summary: Arc<Summary> },
    Io(IoRecord),
}
fn parse_summary(value: &Value) -> Option<Arc<Summary>> {
    serde_json::from_value(value.clone()).ok().map(Arc::new)
}
pub fn parse_data_message(m: &Value) -> Option<DataMessage> {
    match m.get("t")?.as_str()? {
        "data" => Some(DataMessage::Data {
            func: m.get("func")?.as_str()?.into(),
            line: u32::try_from(m.get("line")?.as_u64()?).ok()?,
            var: m.get("var")?.as_str()?.into(),
            n: m.get("n").and_then(Value::as_u64).unwrap_or(1),
            summary: parse_summary(m.get("summary")?)?,
        }),
        "io" => {
            let io = m.get("io")?;
            let calls: Vec<_> = io
                .get("calls")?
                .as_array()?
                .iter()
                .take(20)
                .filter_map(|c| {
                    Some(IoCall {
                        input: c
                            .get("in")?
                            .as_object()?
                            .iter()
                            .filter_map(|(k, v)| parse_summary(v).map(|v| (k.clone(), v)))
                            .collect(),
                        output: c.get("out").and_then(parse_summary),
                    })
                })
                .collect();
            Some(DataMessage::Io(IoRecord {
                func: io.get("func")?.as_str()?.into(),
                total: io.get("total").and_then(Value::as_u64).unwrap_or(calls.len() as u64),
                calls,
            }))
        }
        _ => None,
    }
}
fn num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
fn plural(n: u64, word: &str) -> String {
    format!("{} {word}{}", num(n), if n == 1 { "" } else { "s" })
}
fn rows(s: &Summary) -> u64 {
    s.shape.as_ref().and_then(|s| s.first()).copied().unwrap_or(0)
}
pub fn summary_text(s: &Summary) -> String {
    match s.kind.as_str() {
        "dataframe" => format!(
            "table: {} × {}",
            plural(rows(s), "row"),
            plural(s.shape.as_ref().and_then(|s| s.get(1)).copied().unwrap_or(0), "column")
        ),
        "series" => format!(
            "column: {}{}",
            plural(rows(s), "value"),
            s.dtype.as_ref().filter(|s| !s.is_empty()).map(|d| format!(" ({d})")).unwrap_or_default()
        ),
        "sequence" => format!(
            "{} of {}",
            if matches!(s.type_.as_str(), "tuple" | "set") { &s.type_ } else { "list" },
            num(s.len.unwrap_or(0))
        ),
        "mapping" => format!("dict of {}", num(s.len.unwrap_or(0))),
        _ => s.repr.clone().unwrap_or_else(|| s.type_.clone()),
    }
}
pub fn change(input: Option<&Summary>, output: Option<&Summary>, kind: &str) -> String {
    let Some((a, b)) = input.zip(output).filter(|(a, b)| a.kind == "dataframe" && b.kind == "dataframe") else {
        return String::new();
    };
    let (before, after) = (rows(a), rows(b));
    if kind == "group" {
        return format!("{} → {}", plural(before, "row"), plural(after, "group"));
    }
    if before != after {
        return format!(
            "{} → {} ({}{})",
            num(before),
            plural(after, "row"),
            if after < before { "−" } else { "+" },
            num(before.abs_diff(after))
        );
    }
    let a: Vec<_> = a.columns.iter().flatten().map(|c| c.0.as_str()).collect();
    let b: Vec<_> = b.columns.iter().flatten().map(|c| c.0.as_str()).collect();
    let added: Vec<_> = b.iter().copied().filter(|c| !a.contains(c)).collect();
    let removed: Vec<_> = a.iter().copied().filter(|c| !b.contains(c)).collect();
    if !added.is_empty() {
        format!("+{}: {}", plural(added.len() as u64, "column"), added.join(", "))
    } else if !removed.is_empty() {
        format!("−{}: {}", plural(removed.len() as u64, "column"), removed.join(", "))
    } else {
        String::new()
    }
}
const MAX_POINTS: usize = 20_000;
const MAX_FUNCTIONS: usize = 2_000;
#[derive(Default)]
pub struct DataLog {
    // Nested sorted maps make range queries proportional to captured lines, not
    // the source's line span; Arc summaries are shared with all visible cards.
    lines: BTreeMap<String, BTreeMap<u32, Vec<DataPoint>>>,
    ios: BTreeMap<String, IoRecord>,
    count: usize,
    pub func: Option<String>,
    pub run_id: u64,
    pub dropped: usize,
}
impl DataLog {
    pub fn begin(&mut self, id: u64) {
        *self = Self { run_id: id, ..Default::default() };
    }
    pub fn add(&mut self, m: DataMessage) {
        match m {
            DataMessage::Io(io) => {
                if self.ios.len() < MAX_FUNCTIONS || self.ios.contains_key(&io.func) {
                    self.ios.insert(io.func.clone(), io);
                } else {
                    self.dropped += 1;
                }
            }
            DataMessage::Data { func, line, var, n, summary } => {
                if let Some(point) = self
                    .lines
                    .get_mut(&func)
                    .and_then(|ls| ls.get_mut(&line))
                    .and_then(|ps| ps.iter_mut().find(|p| p.var == var))
                {
                    *point = DataPoint { var, n, summary };
                    return;
                }
                if self.count >= MAX_POINTS {
                    self.dropped += 1;
                    return;
                }
                self.func = Some(func.clone());
                self.lines.entry(func).or_default().entry(line).or_default().push(DataPoint { var, n, summary });
                self.count += 1;
            }
        }
    }
    pub fn between(&self, func: &str, from: u32, to: u32) -> Vec<&DataPoint> {
        if from > to {
            return Vec::new();
        }
        self.lines
            .get(func)
            .into_iter()
            .flat_map(|lines| lines.range(from..=to))
            .flat_map(|(_, points)| points)
            .collect()
    }
    pub fn io(&self, func: &str) -> Option<&IoRecord> {
        self.ios.get(func)
    }
    pub fn has_data(&self) -> bool {
        self.count > 0 || !self.ios.is_empty()
    }
}
