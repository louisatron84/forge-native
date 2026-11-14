//! Schema merging: model columns win over SQL, which wins over SQLite metadata.
use crate::project_graph::RawScan;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DbColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub pk: bool,
    pub unique: bool,
    pub fk: Option<String>,
    pub implicit: bool,
    pub fk_model: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct DbTable {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub framework: Option<String>,
    pub module: Option<String>,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub columns: Vec<DbColumn>,
    pub sources: Vec<String>,
    pub used_by: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct DbRelation {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub column: Option<String>,
}
#[derive(Clone, Debug)]
pub struct DbConnection {
    pub kind: String,
    pub library: String,
    pub target: Option<String>,
    pub module: String,
    pub path: String,
    pub func: Option<String>,
    pub line: usize,
    pub file: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct DbSchema {
    pub tables: Vec<DbTable>,
    pub relations: Vec<DbRelation>,
    pub connections: Vec<DbConnection>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawDb {
    models: Vec<Model>,
    tables: Vec<Table>,
    connections: Vec<Connection>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Model {
    name: String,
    table: String,
    line: usize,
    framework: String,
    columns: Vec<DbColumn>,
    rels: Vec<Rel>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Table {
    name: String,
    line: usize,
    columns: Vec<DbColumn>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Rel {
    kind: String,
    to: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Connection {
    kind: String,
    library: String,
    target: Option<String>,
    func: Option<String>,
    line: usize,
    schema: Option<Schema>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Schema {
    file: String,
    error: Option<String>,
    tables: Vec<Table>,
}
fn add(
    tables: &mut BTreeMap<String, (DbTable, usize)>,
    name: &str,
    source: &str,
    priority: usize,
    columns: Vec<DbColumn>,
) -> String {
    let id = name.to_lowercase();
    let (t, p) = tables
        .entry(id.clone())
        .or_insert_with(|| (DbTable { id: id.clone(), name: name.into(), ..Default::default() }, usize::MAX));
    if !t.sources.iter().any(|s| s == source) {
        t.sources.push(source.into());
        t.sources.sort_by_key(|s| match s.as_str() {
            "model" => 0,
            "sql" => 1,
            _ => 2,
        });
    }
    if !columns.is_empty() && priority < *p {
        t.columns = columns;
        *p = priority;
    }
    id
}
pub fn build_schema(scan: &RawScan) -> DbSchema {
    let mut tables = BTreeMap::new();
    let mut by_class = BTreeMap::new();
    let mut rels = Vec::new();
    let mut out = DbSchema::default();
    for f in &scan.files {
        let Some(db) = f.db.as_ref().and_then(|v| serde_json::from_value::<RawDb>(v.clone()).ok()) else {
            continue;
        };
        for m in db.models {
            let id = add(&mut tables, &m.table, "model", 0, m.columns);
            let t = &mut tables.get_mut(&id).unwrap().0;
            if t.module.is_none() {
                t.module = Some(f.module.clone());
                t.path = Some(f.path.clone());
                t.line = Some(m.line);
                t.model = Some(m.name.clone());
                t.framework = Some(m.framework);
            }
            by_class.insert(m.name, id.clone());
            for r in m.rels {
                rels.push((id.clone(), r));
            }
        }
        for s in db.tables {
            let id = add(&mut tables, &s.name, "sql", 1, s.columns);
            let t = &mut tables.get_mut(&id).unwrap().0;
            if t.module.is_none() {
                t.module = Some(f.module.clone());
                t.path = Some(f.path.clone());
                t.line = Some(s.line);
            }
        }
        for c in db.connections {
            let mut conn = DbConnection {
                kind: c.kind,
                library: c.library,
                target: c.target,
                module: f.module.clone(),
                path: f.path.clone(),
                func: c.func,
                line: c.line,
                file: None,
                error: None,
            };
            if let Some(s) = c.schema {
                conn.file = Some(s.file);
                conn.error = s.error;
                for t in s.tables {
                    add(&mut tables, &t.name, "database", 2, t.columns);
                }
            }
            out.connections.push(conn);
        }
    }
    let names: BTreeMap<_, _> = tables.iter().map(|(id, (t, _))| (id.clone(), t.name.clone())).collect();
    let mut linked = BTreeSet::new();
    for (id, (t, _)) in &mut tables {
        for c in &mut t.columns {
            let target = c
                .fk_model
                .as_ref()
                .and_then(|k| by_class.get(k))
                .cloned()
                .or_else(|| c.fk.as_ref().map(|f| f.to_lowercase()));
            if let Some(to) = target.filter(|s| names.contains_key(s)) {
                c.fk = Some(names[&to].clone());
                out.relations.push(DbRelation {
                    from: id.clone(),
                    to: to.clone(),
                    kind: "fk".into(),
                    column: Some(c.name.clone()),
                });
                linked.insert((id.clone(), to.clone()));
                linked.insert((to, id.clone()));
            }
        }
    }
    for (from, r) in rels {
        let Some(to) = by_class.get(&r.to) else {
            continue;
        };
        if r.kind != "many" && (to == &from || linked.contains(&(from.clone(), to.clone()))) {
            continue;
        }
        out.relations.push(DbRelation {
            from: from.clone(),
            to: to.clone(),
            kind: if r.kind == "many" { "many" } else { "relationship" }.into(),
            column: None,
        });
        linked.insert((from.clone(), to.clone()));
        linked.insert((to.clone(), from));
    }
    for (t, _) in tables.values_mut() {
        if let Some(model) = &t.model {
            for f in &scan.files {
                for d in &f.defs {
                    if matches!(d.kind.as_str(), "function" | "method") && d.refs.contains(model) {
                        let id = format!("{}:{}", f.module, d.qual);
                        if !t.used_by.contains(&id) {
                            t.used_by.push(id);
                        }
                    }
                }
            }
        }
    }
    out.tables = tables.into_values().map(|(t, _)| t).collect();
    out.tables.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
