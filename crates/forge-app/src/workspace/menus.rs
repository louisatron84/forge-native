//! Workspace menus backed by Forge's command registry and editor actions.

use super::Workspace;
use crate::editor;
use forge_core::commands::command;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
    Sizable,
};
use gpui_kit::{div, Action, Context, Focusable, InteractiveElement, IntoElement, ParentElement, Styled};

const MENUS: &[(&str, &[&[&str]])] = &[
    (
        "File",
        &[
            &[
                "file.new",
                "file.newFolder",
                "file.newProject",
                "file.open",
                "file.openFolder",
                "file.openFolderByPath",
                "file.openRecent",
            ],
            &[
                "file.save",
                "file.saveAll",
                "file.undoGenerated",
                "tab.close",
                "file.rename",
                "file.delete",
                "file.saveTemplate",
            ],
            &["settings.open", "settings.project", "keys.show"],
        ],
    ),
    (
        "Edit",
        &[
            &["find.open", "find.replace", "search.project"],
            &[
                "editor.toggleComment",
                "editor.format",
                "editor.formatSelection",
                "editor.organizeImports",
                "editor.fixAll",
                "editor.renameSymbol",
                "editor.quickFix",
            ],
        ],
    ),
    ("Selection", &[]),
    (
        "View",
        &[
            &[
                "palette.commands",
                "view.files",
                "git.show",
                "view.outline",
                "panel.problems",
                "panel.output",
                "panel.debug",
                "tests.show",
            ],
            &["view.projectGraph", "nav.projectMap", "view.pipeline", "panel.network", "panel.memory", "view.database"],
            &[
                "view.split",
                "view.moveToOtherGroup",
                "view.editLayout",
                "view.resetLayout",
                "editor.toggleMinimap",
                "view.zoomIn",
                "view.zoomOut",
            ],
        ],
    ),
    (
        "Go",
        &[
            &["nav.back", "palette.files", "palette.line", "nav.symbol", "nav.projectSymbol"],
            &["nav.definition", "nav.typeDefinition", "nav.implementations", "nav.references", "nav.callHierarchy"],
            &["tab.next", "tab.prev"],
        ],
    ),
    (
        "Run",
        &[
            &["run.smart", "debug.start", "run.file", "run.stop", "debug.restart", "debug.pause"],
            &["debug.stepOver", "debug.stepInto", "debug.stepOut", "debug.toggleBreakpoint"],
            &["run.selectConfig", "run.editConfigs", "run.memory", "tests.runAll", "tests.runFile"],
        ],
    ),
    (
        "Terminal",
        &[
            &["terminal.new", "terminal.newWithShell", "terminal.toggle", "terminal.next", "terminal.kill"],
            &["tasks.run", "tasks.rerun"],
        ],
    ),
    ("Help", &[&["help.welcome", "keys.show", "help.releaseNotes", "help.checkUpdates"]]),
];

fn editor_items(menu: &str) -> Vec<(&'static str, Box<dyn Action>)> {
    match menu {
        "Edit" => vec![
            ("Undo", Box::new(editor::Undo)),
            ("Redo", Box::new(editor::Redo)),
            ("Cut", Box::new(editor::Cut)),
            ("Copy", Box::new(editor::Copy)),
            ("Paste", Box::new(editor::Paste)),
        ],
        "Selection" => vec![
            ("Select All", Box::new(editor::SelectAll)),
            ("Add Cursor Above", Box::new(editor::AddCursorAbove)),
            ("Add Cursor Below", Box::new(editor::AddCursorBelow)),
            ("Add Next Occurrence", Box::new(editor::AddNextOccurrence)),
            ("Select All Occurrences", Box::new(editor::SelectAllOccurrences)),
        ],
        _ => Vec::new(),
    }
}

impl Workspace {
    pub(super) fn render_menu_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<crate::theme::ForgeTheme>().0.clone();
        let workspace = cx.entity().downgrade();
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .px(theme.px(6.0))
            .bg(theme.bg_panel)
            .border_b_1()
            .border_color(theme.border)
            .children(MENUS.iter().map(|&(name, sections)| {
                let workspace = workspace.clone();
                div().debug_selector(move || format!("workspace-menu-{name}")).child(
                    Button::new(name).label(name).ghost().small().dropdown_menu(move |mut menu, window, cx| {
                        let Some(entity) = workspace.upgrade() else { return menu };
                        let ws = entity.read(cx);
                        let has_doc = ws.active_doc().is_some();
                        let editor = &ws.editors[ws.tabs.active_group.min(1)];
                        menu = menu
                            .action_context(editor.focus_handle(cx))
                            .scrollable(true)
                            .max_h(window.viewport_size().height * 0.75);
                        let actions = editor_items(name);
                        let has_actions = !actions.is_empty();
                        for (label, action) in actions {
                            menu = menu.menu_with_disabled(label, action, !has_doc);
                        }
                        for (index, section) in sections.iter().enumerate() {
                            if has_actions || index > 0 {
                                menu = menu.separator();
                            }
                            for &id in *section {
                                let Some(info) = command(id) else { continue };
                                let ws = workspace.clone();
                                let enabled = if id == "editor.renameSymbol" {
                                    has_doc
                                } else {
                                    entity.read(cx).available(id, window, cx)
                                };
                                menu = menu.item(PopupMenuItem::new(info.title).disabled(!enabled).on_click(
                                    move |_, window, cx| {
                                        let _ = ws.update(cx, |ws, cx| {
                                            if id == "editor.renameSymbol" {
                                                ws.focus_editor(window, cx);
                                            }
                                            ws.run_command(id, window, cx);
                                        });
                                    },
                                ));
                            }
                        }
                        menu
                    }),
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_command_resolves_in_the_registry() {
        for &(name, sections) in MENUS {
            assert!(!sections.is_empty() || !editor_items(name).is_empty(), "{name} is empty");
            for id in sections.iter().flat_map(|section| section.iter()) {
                assert!(command(id).is_some(), "{name}: unknown command {id}");
            }
        }
    }
}
