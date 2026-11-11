//! The Settings dialog (Ctrl+,): controls that apply live and save
//! automatically. Replaces `src/renderer/settingsView.ts`.

use crate::app_state;
use crate::palette::corner_mark;
use crate::theme::{ForgeTheme, Theme};
use forge_core::settings::{Density, Formatter, Settings, Theme as ThemeSetting, FONT_SIZE_MAX, FONT_SIZE_MIN};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, AppContext, Context, Entity, EventEmitter, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window,
};
use std::time::Duration;

pub enum SettingsEvent {
    /// Your settings changed (and were saved): apply them.
    Changed,
    /// Run a command (Keyboard Shortcuts…, Project Settings…, pick a shell, check for updates).
    Command(&'static str),
    Closed,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

pub struct SettingsView {
    open: bool,
    python: Entity<InputState>,
    /// Which settings the open project overrides (shown as a note).
    pub project_note: Option<String>,
    /// Settings → Updates: what version this is and what the last check found.
    pub update_info: String,
    /// The name of the terminal shell setting's value (filled in by the workspace).
    pub shell_name: String,
    save_task: Option<Task<()>>,
    _sub: Subscription,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let python =
            cx.new(|cx| InputState::new(window, cx).placeholder("Automatic: project .venv, then python on PATH"));
        let sub = cx.subscribe_in(&python, window, |this, input, e: &InputEvent, _, cx| {
            if let InputEvent::Change = e {
                let value = input.read(cx).value().trim().to_owned();
                // Save a moment after typing stops.
                this.save_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(120)).await;
                    this.update(cx, |this, cx| this.update_settings(cx, |s| s.python_path = value)).ok();
                }));
            }
        });
        Self {
            open: false,
            python,
            project_note: None,
            update_info: String::new(),
            shell_name: "System default".into(),
            save_task: None,
            _sub: sub,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = app_state::settings(cx).python_path.clone();
        self.python.update(cx, |s, cx| s.set_value(path, window, cx));
        self.open = true;
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.emit(SettingsEvent::Closed);
            cx.notify();
        }
    }

    fn update_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        let _ = app_state::update_settings(cx, f);
        cx.emit(SettingsEvent::Changed);
        cx.notify();
    }

    fn row(t: &Theme, label: &'static str, hint: &'static str, control: AnyElement) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::MEDIUM).child(label))
                    .child(div().mt(px(2.0)).text_size(px(12.0)).text_color(t.fg_muted).child(hint)),
            )
            .child(div().flex().flex_none().items_center().gap(px(10.0)).child(control))
            .into_any_element()
    }

    /// A row of buttons, one picked.
    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        t: &Theme,
        options: &[(T, &'static str)],
        value: T,
        set: fn(&mut Settings, T),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .border_1()
            .border_color(t.border)
            .rounded(px(2.0))
            .overflow_hidden()
            .children(options.iter().enumerate().map(|(i, (v, label))| {
                let v = *v;
                let on = v == value;
                div()
                    .id(SharedString::from(format!("{id}-{i}")))
                    .min_w(px(44.0))
                    .px(px(12.0))
                    .py(px(5.0))
                    .flex()
                    .justify_center()
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .when(i > 0, |d| d.border_l_1().border_color(t.border))
                    .when(on, |d| d.bg(t.heat_3).text_color(t.on_heat))
                    .when(!on, |d| d.bg(t.bg).text_color(t.fg_muted).hover(|s| s.text_color(t.fg)))
                    .on_click(cx.listener(move |this, _, _, cx| this.update_settings(cx, |s| set(s, v))))
                    .child(*label)
            }))
            .into_any_element()
    }

    fn checkbox(
        &self,
        id: &'static str,
        t: &Theme,
        on: bool,
        set: fn(&mut Settings, bool),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .w(px(16.0))
            .h(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(if on { t.heat_2 } else { t.line_strong })
            .rounded(px(2.0))
            .text_size(px(11.0))
            .text_color(t.heat_2)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.update_settings(cx, |s| set(s, !on))))
            .child(if on { "✓" } else { "" })
            .into_any_element()
    }

    fn button(t: &Theme, id: &'static str, label: impl Into<SharedString>) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .px(px(10.0))
            .py(px(4.0))
            .border_1()
            .border_color(t.border)
            .rounded(px(2.0))
            .bg(t.bg_panel)
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_hover))
            .child(label.into())
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("settings-closed").into_any_element();
        }
        let t = cx.global::<ForgeTheme>().0.clone();
        let s = app_state::settings(cx).clone();
        let font = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(Self::button(&t, "font-smaller", "−").on_click(cx.listener(|this, _, _, cx| {
                this.update_settings(cx, |s| s.font_size = s.font_size.saturating_sub(1).max(FONT_SIZE_MIN))
            })))
            .child(
                div()
                    .min_w(px(38.0))
                    .flex()
                    .justify_center()
                    .text_color(t.fg_muted)
                    .child(format!("{}px", s.font_size)),
            )
            .child(Self::button(&t, "font-bigger", "+").on_click(cx.listener(|this, _, _, cx| {
                this.update_settings(cx, |s| s.font_size = (s.font_size + 1).min(FONT_SIZE_MAX))
            })))
            .into_any_element();
        let theme = self.segmented(
            "theme",
            &t,
            &[(ThemeSetting::Molten, "Molten"), (ThemeSetting::Plain, "Plain")],
            s.theme,
            |s, v| s.theme = v,
            cx,
        );
        let density = self.segmented(
            "density",
            &t,
            &[(Density::Compact, "Compact"), (Density::Normal, "Normal"), (Density::Roomy, "Roomy")],
            s.density,
            |s, v| s.density = v,
            cx,
        );
        let tab = self.segmented("tab", &t, &[(2u32, "2"), (4, "4"), (8, "8")], s.tab_size, |s, v| s.tab_size = v, cx);
        let formatter = self.segmented(
            "formatter",
            &t,
            &[(Formatter::Auto, "Auto"), (Formatter::Ruff, "Ruff"), (Formatter::Black, "Black")],
            s.formatter,
            |s, v| s.formatter = v,
            cx,
        );
        let python = div()
            .w(px(240.0))
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .child(Input::new(&self.python).appearance(false).cleanable(false).h(px(26.0)))
            .into_any_element();
        let shell = Self::button(&t, "shell", format!("{} ▾", self.shell_name))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Command("settings.pickShell"))))
            .into_any_element();
        let updates = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(self.checkbox("updates", &t, s.check_for_updates, |s, v| s.check_for_updates = v, cx))
            .child(
                Self::button(&t, "check-now", "Check now")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Command("help.checkUpdates")))),
            )
            .into_any_element();
        let rows = vec![
            Self::row(&t, "Font size", "Editor and terminal text", font),
            Self::row(&t, "Theme", "Molten: Forge's colours. Plain: black and white, high contrast", theme),
            Self::row(&t, "Density", "How much fits in the panels: Compact shows more, Roomy is larger and easier to read (the editor follows the font size)", density),
            Self::row(&t, "Animations", "Animate switching tasks and moving panels (off anyway if your system asks for less motion)", self.checkbox("animations", &t, s.animations, |s, v| s.animations = v, cx)),
            Self::row(&t, "Tab width", "Spaces per indent level (Python uses 4)", tab),
            Self::row(&t, "Python interpreter", "Used by Run (F5). Leave empty for automatic.", python),
            Self::row(&t, "Terminal shell", "Used for new terminals", shell),
            Self::row(&t, "Formatter", "For Format Document (Shift+Alt+F). Auto: Ruff if installed, else Black", formatter),
            Self::row(&t, "Format on save", "Format Python files every time they are saved", self.checkbox("format-on-save", &t, s.format_on_save, |s, v| s.format_on_save = v, cx)),
            Self::row(&t, "Ruff warnings", "Show Ruff's lint warnings and quick fixes (when Ruff is installed)", self.checkbox("ruff", &t, s.lint_with_ruff, |s, v| s.lint_with_ruff = v, cx)),
            Self::row(&t, "Debug in terminal", "Run the program being debugged in a terminal, so input() works", self.checkbox("debug-terminal", &t, s.debug_in_terminal, |s, v| s.debug_in_terminal = v, cx)),
            Self::row(&t, "Debug task", "Switch to the Debug task while debugging (and back afterwards)", self.checkbox("debug-task", &t, s.switch_to_debug_task, |s, v| s.switch_to_debug_task = v, cx)),
            Self::row(&t, "Runs on the graph", "When you Run with the Graph in view, show where the program is at (the run is a little slower)", self.checkbox("run-graph", &t, s.show_runs_on_graph, |s, v| s.show_runs_on_graph = v, cx)),
            Self::row(&t, "Reopen last project", "At startup, open the last folder with its tabs and layout", self.checkbox("reopen", &t, s.reopen_last_project, |s, v| s.reopen_last_project = v, cx)),
            Self::row(&t, "Updates", "Keep Forge up to date: download new versions in the background and install them when you close Forge. Off: only when you choose Check for Updates.", updates),
        ];
        let links = div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .mt(px(12.0))
            .child(Self::button(&t, "link-keys", "Keyboard Shortcuts…").on_click(cx.listener(|this, _, _, cx| {
                this.close(cx);
                cx.emit(SettingsEvent::Command("keys.show"));
            })))
            .child(Self::button(&t, "link-project", "Project Settings…").on_click(cx.listener(|this, _, _, cx| {
                this.close(cx);
                cx.emit(SettingsEvent::Command("settings.project"));
            })));
        div()
            .id("settings-overlay")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .items_start()
            .pt(gpui_kit::relative(0.1))
            .bg(t.overlay)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close(cx)))
            .child(
                div()
                    .id("settings-dialog")
                    .relative()
                    .w(px(560.0))
                    .max_w_full()
                    .max_h(gpui_kit::relative(0.8))
                    .overflow_y_scroll()
                    .px(px(20.0))
                    .pt(px(16.0))
                    .pb(px(12.0))
                    .bg(t.bg_panel)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(2.0))
                    .shadow_lg()
                    .text_color(t.fg)
                    .text_size(px(13.0))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(corner_mark(t.heat_3))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb(px(8.0))
                            .child(div().text_size(px(16.0)).font_weight(FontWeight::SEMIBOLD).child("Settings"))
                            .child(
                                div()
                                    .id("settings-close")
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(18.0))
                                    .text_color(t.fg_muted)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.bg_hover).text_color(t.fg))
                                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                                    .child("×"),
                            ),
                    )
                    .when_some(self.project_note.clone(), |d, note| {
                        d.child(
                            div()
                                .mb(px(8.0))
                                .px(px(10.0))
                                .py(px(6.0))
                                .border_l_2()
                                .border_color(t.accent)
                                .bg(t.bg_hover)
                                .text_size(px(12.0))
                                .child(note),
                        )
                    })
                    .children(rows)
                    .when(!self.update_info.is_empty(), |d| {
                        d.child(div().text_size(px(12.0)).text_color(t.fg_muted).child(self.update_info.clone()))
                    })
                    .child(links)
                    .child(
                        div()
                            .mt(px(10.0))
                            .text_size(px(12.0))
                            .text_color(t.fg_muted)
                            .child("Changes apply immediately and are saved automatically."),
                    ),
            )
            .into_any_element()
    }
}

/// The note about the project's own settings file.
pub fn project_note(project_keys: &[&str], has_folder: bool) -> Option<String> {
    let name = |k: &str| {
        match k {
            "tabSize" => "Tab width",
            "pythonPath" => "Python interpreter",
            "shell" => "Terminal shell",
            "formatter" => "Formatter",
            "formatOnSave" => "Format on save",
            "lintWithRuff" => "Ruff warnings",
            other => other,
        }
        .to_owned()
    };
    if !project_keys.is_empty() {
        let names: Vec<String> = project_keys.iter().map(|k| name(k)).collect();
        return Some(format!(
            "This project's .forge/settings.json sets: {}. Those rows show your own setting; the project's value is used while it is open.",
            names.join(", ")
        ));
    }
    has_folder.then(|| "A project can set tabSize, pythonPath, shell, formatter, formatOnSave and lintWithRuff in .forge/settings.json (Project Settings below).".to_owned())
}

#[cfg(test)]
mod tests {
    use super::project_note;

    #[test]
    fn the_note_names_what_the_project_overrides() {
        assert_eq!(project_note(&[], false), None);
        assert!(project_note(&[], true).is_some_and(|n| n.starts_with("A project can set")));
        assert!(
            project_note(&["tabSize", "pythonPath"], true).is_some_and(|n| n.contains("Tab width, Python interpreter"))
        );
    }
}
