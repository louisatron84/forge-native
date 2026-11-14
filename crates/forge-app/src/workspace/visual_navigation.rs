use super::Workspace;
use crate::panels::navigation::NavigationEvent;
use gpui_kit::{Context, Window};
impl Workspace {
    pub(super) fn refresh_navigation_panels(&mut self, cx: &mut Context<Self>) {
        let graph = self.project_graph.read(cx).graph.clone();
        let module = self.active_doc().and_then(|d| {
            let doc = &d.read(cx).doc;
            let root = self.root.as_ref()?;
            let rel = std::path::Path::new(doc.path()).strip_prefix(root).ok()?.to_string_lossy();
            forge_core::navigation::module_for_path(&graph, &rel).map(str::to_owned)
        });
        self.where_panel.update(cx, |p, cx| p.show(graph.clone(), module.clone(), cx));
        self.infile.update(cx, |p, cx| p.show(graph, module, cx));
        if self.project_graph.read(cx).detail_mode == 3 {
            self.refresh_map_markers(cx);
        }
    }
    pub(super) fn refresh_map_markers(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let relative = |path: &str| {
            std::path::Path::new(path).strip_prefix(&root).ok().map(|p| p.to_string_lossy().replace('\\', "/"))
        };
        let current = self.active_doc().and_then(|d| relative(d.read(cx).doc.path()));
        let recent = self
            .tabs
            .docs()
            .into_iter()
            .filter_map(|id| self.docs.get(&id).and_then(|d| relative(d.read(cx).doc.path())))
            .collect();
        let changed = self
            .git_panel
            .read(cx)
            .repo()
            .into_iter()
            .flat_map(|r| {
                r.status
                    .staged
                    .iter()
                    .chain(&r.status.unstaged)
                    .chain(&r.status.conflicts)
                    .filter_map(|f| relative(&r.abs(&f.path).to_string_lossy()))
            })
            .collect();
        let problems = self
            .project_graph
            .read(cx)
            .graph
            .modules
            .iter()
            .filter(|m| !self.diagnostics_for(&root.join(&m.path).to_string_lossy()).is_empty())
            .map(|m| m.path.clone())
            .collect();
        let map = self.project_graph.read(cx).project_map.clone();
        map.update(cx, |p, cx| {
            p.current = current;
            p.recent = recent;
            p.changed = changed;
            p.problems = problems;
            cx.notify();
        });
    }
    pub(super) fn show_project_map(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_panel("graph", cx);
        self.ensure_graph(cx);
        self.project_graph.update(cx, |p, cx| {
            p.detail_mode = 3;
            p.project_map.update(cx, |p, cx| p.focus(window, cx));
            cx.notify();
        });
        self.refresh_map_markers(cx);
    }
    pub(super) fn on_navigation_event(&mut self, event: &NavigationEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            NavigationEvent::CloseMap => self.project_graph.update(cx, |p, cx| {
                p.detail_mode = 0;
                cx.notify();
            }),
            NavigationEvent::OpenModule(module, line) => self.on_graph_event(
                &crate::panels::graph::GraphEvent::Open { module: module.clone(), line: *line },
                window,
                cx,
            ),
            NavigationEvent::OpenFile(path) => {
                let Some(root) = &self.root else {
                    return;
                };
                let Ok(path) = forge_sys::connect::project_path(root, path) else {
                    return;
                };
                self.project_graph.update(cx, |p, cx| {
                    p.detail_mode = 0;
                    cx.notify();
                });
                self.open_path(path, window, cx);
                self.reveal_panel("editor", cx);
                self.focus_editor(window, cx);
            }
        }
    }
}
