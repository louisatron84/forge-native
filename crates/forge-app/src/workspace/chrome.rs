//! The top bar (wordmark, task switcher, project, Run) and the status bar
//! (cursor, indentation, encoding, line endings, language, messages).

use super::Workspace;
use crate::app_state;
use crate::theme::ForgeTheme;
use forge_core::dock::TASKS;
use gpui_kit::{
    div, prelude::FluentBuilder, px, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window,
};

fn task_name(task: &str) -> &'static str {
    match task {
        "connect" => "Connect",
        "debug" => "Debug",
        "run" => "Run",
        _ => "Code",
    }
}

fn task_tip(task: &str) -> &'static str {
    match task {
        "connect" => "Connect: the project graph (Ctrl+Alt+2)",
        "debug" => "Debug (Ctrl+Alt+3)",
        "run" => "Run: output and tests (Ctrl+Alt+4)",
        _ => "Code (Ctrl+Alt+1)",
    }
}

impl Workspace {
    pub(super) fn render_toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let current = self.tasks.current();
        let tasks = TASKS.iter().map(|task| {
            let on = *task == current;
            let task: &'static str = task;
            div()
                .id(SharedString::from(format!("task-{task}")))
                .px(px(12.0))
                .py(px(3.0))
                .border_1()
                .rounded(px(2.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(on, |d| d.text_color(t.heat_2).border_color(t.heat_2).border_b_2())
                .when(!on, |d| {
                    d.text_color(t.fg_muted).border_color(t.border).hover(|s| s.text_color(t.fg).bg(t.bg_hover))
                })
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(task_tip(task)).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| this.run_command(&format!("task.{task}"), window, cx)))
                .child(task_name(task))
        });
        let folder = self.root.as_ref().map(|r| r.to_string_lossy().into_owned());
        let busy = self.is_running(cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.0))
            .px(px(10.0))
            .py(px(6.0))
            .bg(t.bg_panel)
            .border_b_1()
            .border_color(t.border)
            .child(
                div().text_size(px(15.0)).font_weight(FontWeight::BOLD).text_color(t.heat_2).mr(px(6.0)).child("Forge"),
            )
            .child(div().flex().gap(px(4.0)).ml(px(6.0)).children(tasks))
            .child(div().flex_1())
            .child(
                div()
                    .max_w(gpui_kit::relative(0.4))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(12.0))
                    .text_color(if folder.is_some() { t.fg } else { t.fg_muted })
                    .child(folder.unwrap_or_else(|| "No folder open".into())),
            )
            .child(
                div()
                    .id("run-button")
                    .px(px(14.0))
                    .py(px(5.0))
                    .bg(t.heat_3)
                    .text_color(t.on_heat)
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.heat_2))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Run (F5 / Ctrl+F5)").build(window, cx)
                    })
                    .when(busy, |d| d.bg(t.err))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_command(if busy { "run.stop" } else { "run.smart" }, window, cx)
                    }))
                    .child(if busy { "■ Stop" } else { "▶ Run" }),
            )
    }

    pub(super) fn render_status_bar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut right: Vec<(&'static str, String, Option<&'static str>)> = Vec::new();
        if let Some(doc) = self.active_doc() {
            let ed = self.editors[self.tabs.active_group.min(1)].read(cx);
            let c = ed.cursor();
            let cursors = ed.cursor_count();
            let selected = ed.selected_text(cx).chars().count();
            let d = &doc.read(cx).doc;
            let extra = if cursors > 1 {
                format!(" ({cursors} cursors)")
            } else if selected > 0 {
                format!(" ({selected} selected)")
            } else {
                String::new()
            };
            let lang = d.mode().language;
            let tab_size = app_state::settings(cx).tab_size;
            right.push(("sb-cursor", format!("Ln {}, Col {}{extra}", c.line + 1, c.col + 1), Some("palette.line")));
            right.push(("sb-indent", format!("Spaces: {tab_size}"), Some("settings.open")));
            right.push(("sb-encoding", d.encoding.label().to_owned(), Some("file.saveEncoding")));
            right.push(("sb-eol", d.eol().label().to_owned(), Some("file.lineEndings")));
            let lang_text = if d.large && lang.server.is_some() {
                format!("{} (large file: no error checking)", lang.name)
            } else {
                lang.name.to_owned()
            };
            right.push(("sb-lang", lang_text, None));
        }
        let item = |id: &'static str, text: String, command: Option<&'static str>, cx: &mut Context<Self>| {
            div()
                .id(id)
                .px(px(7.0))
                .py(px(2.0))
                .rounded(px(2.0))
                .whitespace_nowrap()
                .when_some(command, |d, command| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(t.bg_hover).text_color(t.fg))
                        .on_click(cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)))
                })
                .child(text)
        };
        let items: Vec<_> = right.into_iter().map(|(id, text, command)| item(id, text, command, cx)).collect();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .min_h(px(24.0))
            .px(px(8.0))
            .bg(t.bg_panel)
            .border_t_1()
            .border_color(t.border)
            .text_color(t.fg_muted)
            .text_size(px(12.0))
            .when(self.root.is_some(), |d| {
                let label = format!("▶ {}", self.run.config.clone().unwrap_or_else(|| "Current file".into()));
                d.child(
                    div()
                        .id("sb-run")
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(2.0))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|s| s.bg(t.bg_hover).text_color(t.fg))
                        .on_click(cx.listener(|this, _, window, cx| this.run_command("run.selectConfig", window, cx)))
                        .child(label),
                )
            })
            .child(div().flex_1())
            .when_some(self.note.clone(), |d, note| {
                d.child(div().px(px(7.0)).min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(note))
            })
            .children(items)
    }
}
