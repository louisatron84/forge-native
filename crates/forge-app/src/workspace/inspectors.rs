use super::Workspace;
use crate::panels::diagram::{DiagramEdge, DiagramEvent, DiagramNode};
use forge_core::database::DbSchema;
use gpui_kit::{Context, Window};
impl Workspace {
    pub(super) fn set_database(&mut self, schema: DbSchema, cx: &mut Context<Self>) {
        let mut nodes = schema
            .tables
            .iter()
            .map(|t| DiagramNode {
                id: t.id.clone(),
                title: format!("{}{}", t.name, t.model.as_ref().map(|m| format!(" · {m}")).unwrap_or_default()),
                fields: t
                    .columns
                    .iter()
                    .map(|c| {
                        format!(
                            "{}{}: {}{}{}",
                            if c.pk { "◆ " } else { "" },
                            c.name,
                            c.type_,
                            if c.unique { " UNIQUE" } else { "" },
                            c.fk.as_ref().map(|f| format!(" → {f}")).unwrap_or_default()
                        )
                    })
                    .chain(std::iter::once(format!("Sources: {}", t.sources.join(", "))))
                    .collect(),
                source: t.path.as_ref().map(|p| (p.clone(), t.line.unwrap_or(1).saturating_sub(1))),
                functions: t.used_by.clone(),
            })
            .collect::<Vec<_>>();
        let edges = schema
            .relations
            .iter()
            .map(|r| DiagramEdge {
                from: r.from.clone(),
                to: r.to.clone(),
                label: format!("{}{}", r.kind, r.column.as_ref().map(|c| format!(" ({c})")).unwrap_or_default()),
            })
            .collect();
        for (i, c) in schema.connections.iter().enumerate() {
            nodes.push(DiagramNode {
                id: format!("connection:{i}"),
                title: format!("{} · {}", c.kind, c.library),
                fields: vec![
                    c.target.clone().unwrap_or_else(|| "Dynamic connection".into()),
                    c.error.clone().unwrap_or_default(),
                    c.file.clone().unwrap_or_default(),
                ],
                source: Some((c.path.clone(), c.line.saturating_sub(1))),
                ..Default::default()
            });
        }
        self.database.update(cx, |p, cx| {
            p.status = if nodes.is_empty() {
                "Looks for SQLAlchemy, SQLModel, Django and Peewee models, SQL strings, and project SQLite files."
                    .into()
            } else {
                String::new()
            };
            p.set_data(nodes, edges, cx);
        });
    }
    pub(super) fn on_diagram_event(&mut self, event: &DiagramEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            DiagramEvent::Refresh => {
                self.queue_graph_scan(None, cx);
            }
            DiagramEvent::Open(path, line) => {
                let Some(root) = &self.root else {
                    return;
                };
                let Ok(path) = forge_sys::connect::project_path(root, path) else {
                    return;
                };
                self.open_path_at(path, Some(*line), window, cx);
                self.reveal_panel("editor", cx);
                self.focus_editor(window, cx);
            }
            DiagramEvent::Function(id) => {
                let graph = &self.project_graph.read(cx).graph;
                let found = graph.modules.iter().find_map(|m| {
                    m.symbols.iter().find(|s| &s.id == id).map(|s| (m.id.clone(), s.line.saturating_sub(1)))
                });
                if let Some((module, line)) = found {
                    self.on_graph_event(&crate::panels::graph::GraphEvent::Open { module, line }, window, cx);
                }
            }
        }
    }
}
