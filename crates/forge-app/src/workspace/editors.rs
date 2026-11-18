//! The Editor panel: one or two groups side by side, each a row of tabs over
//! an editor, and the Welcome page when no file is open. Replaces
//! `src/renderer/tabBar.ts` and `src/renderer/welcome.ts`.

use super::Workspace;
use crate::app_state;
use crate::palette::corner_mark;
use crate::theme::{ForgeTheme, Theme};
use forge_core::tabs::{same_path, DocId, TabGroup};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window,
};

/// A link on the Welcome page: label, command, and a note.
struct Action {
    label: &'static str,
    command: &'static str,
    detail: Option<&'static str>,
}

const fn act(label: &'static str, command: &'static str, detail: Option<&'static str>) -> Action {
    Action { label, command, detail }
}

fn folder_name(p: &str) -> String {
    p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(p).to_owned()
}

impl Workspace {
    /// The Welcome page shows when no file is open (if the settings say so), or when asked for.
    pub(crate) fn welcome_visible(&self, cx: &gpui_kit::App) -> bool {
        self.welcome_forced || (self.tabs.docs().is_empty() && app_state::settings(cx).show_welcome)
    }

    pub(super) fn render_editor_area(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.git_inspect.read(cx).open {
            return div().size_full().child(self.git_inspect.clone()).into_any_element();
        }
        let t = cx.global::<ForgeTheme>().0.clone();
        if self.welcome_visible(cx) {
            return self.render_welcome(window, cx);
        }
        let groups = self.tabs.groups.clone();
        let active_group = self.tabs.active_group;
        let split = groups.len() > 1;
        div()
            .size_full()
            .flex()
            .min_w_0()
            .min_h_0()
            .children(groups.iter().enumerate().map(|(i, g)| {
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .when(i > 0, |d| d.border_l_1().border_color(t.border))
                    .child(self.render_tab_row(i, g, split && i == active_group, &t, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .when(g.docs.is_empty(), |d| {
                                d.flex().items_center().justify_center().text_color(t.fg_muted).child("No file open")
                            })
                            .when(!g.docs.is_empty(), |d| d.child(self.editors[i.min(1)].clone())),
                    )
                    .when(i == active_group, |d| d.child(self.find_bar.clone()))
            }))
            .into_any_element()
    }

    fn render_tab_row(
        &self,
        group: usize,
        g: &TabGroup,
        focused_side: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tabs = g.docs.iter().enumerate().filter_map(|(i, id)| {
            let doc = self.docs.get(id)?;
            let d = &doc.read(cx).doc;
            let (name, path, dirty) = (d.name().to_owned(), d.path().to_owned(), d.dirty());
            let active = g.active == Some(*id);
            let id = *id;
            let group_name = SharedString::from(format!("tab-{group}-{i}"));
            Some(
                div()
                    .id(SharedString::from(format!("tab-{group}-{id}")))
                    .group(group_name.clone())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(12.0))
                    .pr(px(6.0))
                    .rounded(t.corner(px(3.0)))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.raised)
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .text_color(if active { t.fg } else { t.fg_muted })
                    .when(active, |d| d.bg(t.bg).border_b_2().border_color(t.heat_2))
                    .when(!active, |d| d.hover(|s| s.bg(t.bg_hover)))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tabs.activate(id, Some(group));
                        this.after_tabs_changed(window, cx);
                        this.focus_editor(window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab(id, Some(group), window, cx)),
                    )
                    .child(name)
                    .child(
                        div()
                            .id(SharedString::from(format!("tab-close-{group}-{id}")))
                            .w(px(20.0))
                            .h(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(t.corner(px(2.0)))
                            .text_size(px(if dirty { 10.0 } else { 14.0 }))
                            .hover(|s| s.bg(t.bg_active))
                            .when(!dirty && !active, |d| d.invisible().group_hover(group_name, |s| s.visible()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(id, Some(group), window, cx);
                            }))
                            .child(if dirty { "●" } else { "×" }),
                    ),
            )
        });
        div()
            .id(("tab-row", group))
            .flex()
            .flex_none()
            .h(px(34.0))
            .gap(px(2.0))
            .px(px(3.0))
            .py(px(2.0))
            .overflow_x_scroll()
            .bg(t.bg_panel)
            .border_b_1()
            .border_color(if focused_side { t.heat_3 } else { t.border })
            .children(tabs)
    }

    // ---------- the Welcome page ----------

    fn render_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let folder = self.root.as_ref().map(|r| r.to_string_lossy().into_owned());
        let start = [
            act(
                "New File…",
                "file.new",
                Some(if folder.is_some() {
                    "in the project (type a/b.py to make folders too)"
                } else {
                    "choose where to save it"
                }),
            ),
            act(
                "New Project Folder…",
                "file.newFolder",
                Some(if folder.is_some() { "in the project" } else { "a new project folder, then open it" }),
            ),
            act(
                "New Project from Template…",
                "file.newProject",
                Some("a script, web API, data analysis, desktop app… or one of yours"),
            ),
            act("Open Folder…", "file.openFolder", None),
            act("Open Folder by Path…", "file.openFolderByPath", Some("type or paste a path")),
        ];
        let project = [
            act("Go to File…", "palette.files", None),
            act("Project Graph", "view.projectGraph", Some("how the modules fit together")),
            act("Search in Project", "search.project", None),
            act("Run Task…", "tasks.run", None),
            act("Run Configurations…", "run.selectConfig", None),
            act("Run All Tests", "tests.runAll", None),
            act("Python Interpreter…", "python.selectInterpreter", None),
            act("Python Packages…", "python.packages", None),
            act("Project Settings", "settings.project", None),
        ];
        let shortcuts = [
            act("Show All Commands", "palette.commands", None),
            act("Go to File", "palette.files", None),
            act("Run / Debug", "run.smart", None),
            act("Toggle Terminal", "terminal.toggle", None),
            act("Split Editor", "view.split", None),
            act("Settings", "settings.open", None),
            act("Keyboard Shortcuts…", "keys.show", None),
        ];
        let left = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(260.0))
            .gap(px(18.0))
            .child(self.welcome_section("welcome-start", "START", &start, false, window, cx))
            .child(self.welcome_recent(folder.as_deref(), &t, cx));
        let right = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(260.0))
            .gap(px(18.0))
            .when(folder.is_some(), |d| {
                d.child(self.welcome_section("welcome-project", "THIS PROJECT", &project, false, window, cx))
            })
            .child(self.welcome_section("welcome-keys", "USEFUL SHORTCUTS", &shortcuts, true, window, cx));
        let show = app_state::settings(cx).show_welcome;
        let can_close = !self.tabs.docs().is_empty();
        div()
            .id("welcome")
            .size_full()
            .overflow_y_scroll()
            .bg(t.bg)
            .when_some(self.release_notes.clone(), |d, release| {
                d.child(
                    div()
                        .id("welcome-whats-new")
                        .p(px(16.0))
                        .bg(t.bg_panel)
                        .cursor_pointer()
                        .child(format!(
                            "What's new in Forge {} — {}",
                            release.version,
                            release.notes.lines().next().unwrap_or("Read release notes")
                        ))
                        .on_click(cx.listener(|this, _, window, cx| this.show_release_notes(window, cx))),
                )
            })
            .child(
                div()
                    .max_w(px(860.0))
                    .mx_auto()
                    .px(px(32.0))
                    .py(px(28.0))
                    .flex()
                    .flex_col()
                    .gap(px(22.0))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(16.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(30.0))
                                            .font_weight(gpui_kit::FontWeight::BOLD)
                                            .text_color(t.heat_2)
                                            .child("Forge"),
                                    )
                                    .child(div().text_color(t.fg_muted).child(match &folder {
                                        Some(f) => format!("Working in {}", folder_name(f)),
                                        None => "A hand-written IDE for Python".to_owned(),
                                    })),
                            )
                            .child(div().flex_1())
                            .when(can_close, |d| {
                                d.child(
                                    div()
                                        .id("welcome-close")
                                        .px(px(10.0))
                                        .py(px(4.0))
                                        .border_1()
                                        .border_color(t.border)
                                        .rounded(t.corner(px(2.0)))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(t.bg_hover))
                                        .on_click(cx.listener(|this, _, window, cx| this.close_welcome(window, cx)))
                                        .child("× Back to your files"),
                                )
                            }),
                    )
                    .child(div().flex().flex_wrap().gap(px(32.0)).child(left).child(right))
                    .child(
                        div()
                            .id("welcome-show")
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .text_color(t.fg_muted)
                            .text_size(px(12.0))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = app_state::update_settings(cx, |s| s.show_welcome = !show);
                                cx.notify();
                                let _ = this;
                            }))
                            .child(
                                div()
                                    .w(px(14.0))
                                    .h(px(14.0))
                                    .border_1()
                                    .border_color(t.line_strong)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(10.0))
                                    .text_color(t.heat_2)
                                    .child(if show { "✓" } else { "" }),
                            )
                            .child("Show this page when no files are open (Help → Welcome always shows it)"),
                    ),
            )
            .into_any_element()
    }

    fn welcome_section(
        &self,
        id: &'static str,
        title: &'static str,
        actions: &[Action],
        show_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let keymap = app_state::state(cx).keymap.clone();
        let rows: Vec<AnyElement> = actions
            .iter()
            .enumerate()
            .filter(|(_, a)| keymap.get(a.command).is_some())
            .map(|(i, a)| {
                let usable = self.available(a.command, window, cx);
                let key = keymap.keys_of(a.command).into_iter().next();
                let command = a.command;
                div()
                    .id(SharedString::from(format!("{id}-{i}")))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(3.0))
                    .px(px(4.0))
                    .rounded(t.corner(px(2.0)))
                    .when(usable || show_keys, |d| d.cursor_pointer().hover(|s| s.bg(t.bg_hover)))
                    .when(!usable && !show_keys, |d| d.opacity(0.45))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.available(command, window, cx) {
                            this.run_command(command, window, cx);
                        }
                    }))
                    .child(div().flex_none().text_color(t.link).child(a.label))
                    .when_some(a.detail.filter(|_| !show_keys), |d, detail| {
                        d.child(div().flex_1().min_w_0().text_size(px(12.0)).text_color(t.fg_muted).child(detail))
                    })
                    .when_some(key, |d, k| {
                        d.child(
                            div()
                                .ml_auto()
                                .px(px(6.0))
                                .py(px(1.0))
                                .border_1()
                                .border_color(t.border)
                                .rounded(t.corner(px(2.0)))
                                .text_size(px(11.0))
                                .text_color(t.fg_muted)
                                .child(k),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        section(&t, title, rows)
    }

    fn welcome_recent(&self, current: Option<&str>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let all = app_state::state(cx).user.recent();
        let list: Vec<String> =
            all.iter().filter(|f| current.is_none_or(|c| !same_path(f, c))).take(8).cloned().collect();
        let mut rows: Vec<AnyElement> = Vec::new();
        if list.is_empty() {
            rows.push(
                div()
                    .text_size(px(12.0))
                    .text_color(t.fg_muted)
                    .child(if current.is_some() {
                        "Other folders you open will be listed here."
                    } else {
                        "Folders you open will be listed here."
                    })
                    .into_any_element(),
            );
        }
        for (i, folder) in list.iter().enumerate() {
            let (f1, f2) = (folder.clone(), folder.clone());
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .id(("welcome-recent", i))
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(10.0))
                            .py(px(3.0))
                            .px(px(4.0))
                            .rounded(t.corner(px(2.0)))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.bg_hover))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_recent(f1.clone(), window, cx)))
                            .child(div().flex_none().text_color(t.link).child(folder_name(folder)))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(px(12.0))
                                    .text_color(t.fg_muted)
                                    .child(folder.clone()),
                            ),
                    )
                    .child(
                        div()
                            .id(("welcome-recent-x", i))
                            .px(px(6.0))
                            .text_color(t.fg_muted)
                            .cursor_pointer()
                            .hover(|s| s.text_color(t.err))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = app_state::state(cx).user.remove_recent(&f2);
                                this.refresh_recent(cx);
                            }))
                            .child("×"),
                    )
                    .into_any_element(),
            );
        }
        if all.len() > list.len() + usize::from(current.is_some()) {
            rows.push(
                div()
                    .id("welcome-more")
                    .py(px(3.0))
                    .px(px(4.0))
                    .text_color(t.link)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.run_command("file.openRecent", window, cx)))
                    .child("More…")
                    .into_any_element(),
            );
        }
        section(t, "RECENT", rows)
    }

    pub(crate) fn close_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.welcome_forced = false;
        self.focus_editor(window, cx);
        cx.notify();
    }

    /// The file a tab shows (for commands acting on "the active file").
    pub(crate) fn active_id(&self) -> Option<DocId> {
        self.tabs.active()
    }
}

/// A Welcome page section: a small heading and its rows, with the corner mark.
fn section(t: &Theme, title: &'static str, rows: Vec<AnyElement>) -> impl IntoElement {
    div()
        .relative()
        .pl(px(12.0))
        .pt(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(corner_mark(t.heat_3))
        .child(div().text_size(px(10.0)).text_color(t.fg_muted).mb(px(6.0)).child(title))
        .children(rows)
}
