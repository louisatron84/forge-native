//! Memory snapshots from the unchanged tracer. All histories have retention caps.
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct MemSite {
    pub path: String,
    pub line: u32,
    pub bytes: f64,
    pub count: f64,
}
#[derive(Clone, Debug, Deserialize)]
pub struct MemKept {
    #[serde(rename = "type")]
    pub type_: String,
    pub count: f64,
    pub path: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct MemMessage {
    pub samples: Vec<(f64, f64)>,
    pub peak: f64,
    pub sites: Option<Vec<MemSite>>,
    pub types: Option<Vec<(String, f64)>>,
    pub final_: bool,
    pub kept: Option<Vec<MemKept>>,
}
fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= 0.)
}
pub fn parse_mem(v: &Value) -> Option<MemMessage> {
    if v.get("t")?.as_str()? != "mem" {
        return None;
    }
    Some(MemMessage {
        peak: num(v.get("peak")?)?,
        samples: v
            .get("samples")?
            .as_array()?
            .iter()
            .filter_map(|s| Some((num(s.get(0)?)?, num(s.get(1)?)?)))
            .collect(),
        sites: v.get("sites").and_then(Value::as_array).map(|ss| {
            ss.iter()
                .filter_map(|s| {
                    Some(MemSite {
                        path: s.get(0)?.as_str()?.into(),
                        line: u32::try_from(s.get(1)?.as_u64()?).ok()?,
                        bytes: num(s.get(2)?)?,
                        count: num(s.get(3)?)?,
                    })
                })
                .collect()
        }),
        types: v
            .get("types")
            .and_then(Value::as_array)
            .map(|ss| ss.iter().filter_map(|s| Some((s.get(0)?.as_str()?.into(), num(s.get(1)?)?))).collect()),
        kept: v.get("kept").and_then(Value::as_array).map(|ss| {
            ss.iter()
                .filter_map(|s| {
                    serde_json::from_value::<MemKept>(s.clone()).ok().filter(|k| k.count.is_finite() && k.count >= 0.)
                })
                .collect()
        }),
        final_: v.get("final").and_then(Value::as_bool) == Some(true),
    })
}
#[derive(Default)]
pub struct MemoryLog {
    pub run_id: u64,
    pub samples: Vec<(f64, f64)>,
    pub peak: f64,
    pub sites: Vec<MemSite>,
    pub types: BTreeMap<String, Vec<(f64, f64)>>,
    pub kept: Vec<MemKept>,
    pub running: bool,
}
impl MemoryLog {
    pub fn begin(&mut self, id: u64) {
        *self = Self { run_id: id, running: true, ..Default::default() };
    }
    pub fn add(&mut self, m: MemMessage) {
        let at = m.samples.first().map(|s| s.0).or_else(|| self.samples.last().map(|s| s.0)).unwrap_or(0.);
        self.samples.extend(m.samples);
        if self.samples.len() > 6000 {
            self.samples.drain(..self.samples.len() - 6000);
        }
        self.peak = self.peak.max(m.peak);
        if let Some(s) = m.sites {
            self.sites = s.into_iter().take(100).collect();
        }
        if let Some(types) = m.types {
            for (name, count) in types {
                if self.types.len() >= 1000 && !self.types.contains_key(&name) {
                    continue;
                }
                let list = self.types.entry(name).or_default();
                list.push((at, count));
                if list.len() > 6000 {
                    list.remove(0);
                }
            }
        }
        if let Some(k) = m.kept {
            self.kept = k.into_iter().take(1000).collect();
        }
        if m.final_ {
            self.running = false;
        }
    }
    pub fn count(&self, name: &str) -> f64 {
        self.types.get(name).and_then(|v| v.last()).map_or(0., |s| s.1)
    }
    pub fn growing(&self, name: &str) -> bool {
        let Some(list) = self.types.get(name).filter(|l| l.len() >= 3) else {
            return false;
        };
        let (first, last) = (list.first().unwrap().1, list.last().unwrap().1);
        last >= first * 2. && last - first >= 20. && list.windows(2).all(|s| s[1].1 >= s[0].1 * 0.9)
    }
}
pub fn format_bytes(n: f64) -> String {
    if n < 1024. {
        format!("{n} B")
    } else if n < 1024. * 1024. {
        format!("{:.1} KB", n / 1024.)
    } else if n < 100. * 1024. * 1024. {
        format!("{:.1} MB", n / 1024. / 1024.)
    } else {
        format!("{:.0} MB", n / 1024. / 1024.)
    }
}
