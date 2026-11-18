use super::{generated::NewFiles, Workspace};
use crate::{app_state, palette::Choice};
use forge_core::templates::{builtin_templates, fill, render_template, ProjectTemplate, TemplateFile};
use gpui_kit::{Context, Window};

fn contents(files: Vec<TemplateFile>) -> Vec<(String, Vec<u8>)> {
    files.into_iter().map(|file| (file.path, file.content.into_bytes())).collect()
}

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
                        "Preview",
                        move |ws, name, window, cx| {
                            let files = render_template(&template, &name);
                            let project = match forge_sys::templates::project_target(&parent, &name, &files) {
                                Ok(project) => project,
                                Err(error) => return ws.alert(error.to_string(), window, cx),
                            };
                            let current = root.clone();
                            let open = template.open.as_ref().map(|s| fill(s, &name));
                            ws.review_new_files(
                                NewFiles {
                                    directory: project.clone(),
                                    files: contents(files),
                                    source: "Project template",
                                    exclusive: true,
                                },
                                move |summary, ws, window, cx| {
                                    if summary.written == 0 {
                                        return;
                                    }
                                    let project = forge_sys::files::canonical_path(&project);
                                    if ws.root != current {
                                        return ws.status_note(format!("Created {}", project.display()), cx);
                                    }
                                    ws.open_folder(project.clone(), window, cx);
                                    if ws.root.as_ref() == Some(&project) {
                                        if let Some(open) = open
                                            .and_then(|rel| forge_sys::connect::project_path(&project, &rel).ok())
                                            .filter(|p| p.is_file())
                                        {
                                            ws.open_path(open, window, cx);
                                        }
                                    }
                                },
                                window,
                                cx,
                            );
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
                "Preview",
                move |_, name, window, cx| {
                    let root = root.clone();
                    let store = store.clone();
                    cx.spawn_in(window, async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move { forge_sys::templates::plan_save_template(&root, &store, &name, "") })
                            .await;
                        this.update_in(cx, |ws, window, cx| {
                            let (directory, files) = match result {
                                Ok(prepared) => prepared,
                                Err(error) => return ws.alert(error.to_string(), window, cx),
                            };
                            ws.review_new_files(
                                NewFiles {
                                    directory,
                                    files: contents(files),
                                    source: "Save project template",
                                    exclusive: true,
                                },
                                |summary, ws, _, cx| {
                                    if summary.written > 0 && !summary.failed {
                                        ws.status_note("Saved the approved template files.", cx);
                                    }
                                },
                                window,
                                cx,
                            );
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
