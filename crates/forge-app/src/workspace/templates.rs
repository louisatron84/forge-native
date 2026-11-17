use super::Workspace;
use crate::{app_state, palette::Choice};
use forge_core::templates::{builtin_templates, fill, render_template, ProjectTemplate};
use gpui_kit::{Context, Window};
impl Workspace {
    pub(super) fn choose_project_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = app_state::state(cx).user.dir().join("templates");
        cx.spawn_in(window, async move |this, cx| {
            let templates = cx
                .background_executor()
                .spawn(async move {
                    let mut templates = builtin_templates();
                    templates.extend(forge_sys::templates::list_user_templates(&store));
                    templates
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                let choices = templates
                    .into_iter()
                    .map(|t| {
                        let label = t.name.clone();
                        let detail = t.detail.clone();
                        Choice::new(label, move |ws, window, cx| ws.create_template_project(t.clone(), window, cx))
                            .detail(detail)
                    })
                    .collect();
                ws.choose("New project from template", choices, window, cx);
            })
            .ok();
        })
        .detach();
    }
    fn create_template_project(&mut self, template: ProjectTemplate, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            directories: true,
            files: false,
            multiple: false,
            prompt: Some("Choose the parent folder for the new project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(parent) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |ws, window, cx| {
                let template = template.clone();
                let root = ws.root.clone();
                ws.palette.update(cx, |p, cx| {
                    p.prompt(
                        "New project name",
                        "",
                        "Create",
                        move |_, name, window, cx| {
                            let files = render_template(&template, &name);
                            let parent = parent.clone();
                            let root = root.clone();
                            let open = template.open.as_ref().map(|s| fill(s, &name));
                            cx.spawn_in(window, async move |this, cx| {
                                let result = cx
                                    .background_executor()
                                    .spawn(async move { forge_sys::templates::create_project(&parent, &name, &files) })
                                    .await;
                                this.update_in(cx, |ws, window, cx| match result {
                                    Ok(project) => {
                                        let project = forge_sys::files::canonical_path(&project);
                                        if ws.root != root {
                                            ws.status_note(format!("Created {}", project.display()), cx);
                                            return;
                                        }
                                        ws.open_folder(project.clone(), window, cx);
                                        if ws.root.as_ref() == Some(&project) {
                                            if let Some(open) = open
                                                .and_then(|rel| forge_sys::connect::project_path(&project, &rel).ok())
                                            {
                                                ws.open_path(open, window, cx);
                                            }
                                        }
                                    }
                                    Err(e) => ws.alert(e.to_string(), window, cx),
                                })
                                .ok();
                            })
                            .detach();
                        },
                        window,
                        cx,
                    )
                });
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn save_project_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if !self.save_all_dirty(window, cx) {
            return;
        }
        let store = app_state::state(cx).user.dir().join("templates");
        self.palette.update(cx, |p, cx| {
            p.prompt(
                "Save project as template",
                "",
                "Save",
                move |_, name, window, cx| {
                    let root = root.clone();
                    let store = store.clone();
                    cx.spawn_in(window, async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move { forge_sys::templates::save_template(&root, &store, &name, "") })
                            .await;
                        this.update(cx, |ws, cx| match result {
                            Ok(_) => ws.status_note(
                                "Project template saved. It is available in New Project from Template.",
                                cx,
                            ),
                            Err(e) => ws.status_note(e.to_string(), cx),
                        })
                        .ok();
                    })
                    .detach();
                },
                window,
                cx,
            )
        });
    }
}
