//! Native debugger controls, sessions, stack, variables, watches and console.
//! Replaces `src/renderer/debug/debugView.ts` and `debugConsole.ts`.
use crate::theme::ForgeTheme;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, AnyElement, AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
};

#[derive(Clone)]
pub enum DebugEvent {
    Command(&'static str),
    Session(u64),
    Thread(i64),
    Frame(i64),
    Expand(i64, usize),
    Evaluate(String),
    AddWatch,
    RemoveWatch(usize),
    Watch(usize),
    ToggleBreakpoint(String, usize),
    EditBreakpoint(String, usize),
    ClearConsole,
}

#[derive(Clone)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub reference: i64,
    pub depth: usize,
}

pub struct DebugPanel {
    pub status: String,
    pub sessions: Vec<(u64, String)>,
    pub active_session: u64,
    pub threads: Vec<(i64, String)>,
    pub frames: Vec<(i64, String)>,
    pub variables: Vec<Variable>,
    pub watches: Vec<(String, String, i64)>,
    pub breakpoints: Vec<(String, usize, String)>,
    pub console: Vec<String>,
    input: Entity<InputState>,
    _sub: Subscription,
}

impl EventEmitter<DebugEvent> for DebugPanel {}

impl DebugPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Debug console: evaluate an expression"));
        let sub = cx.subscribe_in(&input, window, |_, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = input.read(cx).value().trim().to_owned();
                if !text.is_empty() {
                    input.update(cx, |s, cx| s.set_value("", window, cx));
                    cx.emit(DebugEvent::Evaluate(text));
                }
            }
        });
        Self {
            status: "Not debugging".into(),
            sessions: Vec::new(),
            active_session: 0,
            threads: Vec::new(),
            frames: Vec::new(),
            variables: Vec::new(),
            watches: Vec::new(),
            breakpoints: Vec::new(),
            console: Vec::new(),
            input,
            _sub: sub,
        }
    }
    pub fn append(&mut self, text: &str, cx: &mut Context<Self>) {
        self.console.extend(text.lines().map(str::to_owned));
        if self.console.len() > 2000 {
            self.console.drain(..self.console.len() - 2000);
        }
        cx.notify();
    }
    fn row(&self, id: impl Into<SharedString>, label: String, event: DebugEvent, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        div()
            .id(id.into())
            .px(t.px(8.0))
            .py(t.px(2.0))
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_hover))
            .child(label)
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone())))
            .into_any_element()
    }
}

impl Render for DebugPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        let header = |title: &str| {
            div()
                .px(t.px(8.0))
                .pt(t.px(10.0))
                .pb(t.px(4.0))
                .text_color(t.fg_muted)
                .child(title.to_owned())
                .into_any_element()
        };
        rows.push(header(&self.status));
        for (id, name) in &self.sessions {
            rows.push(self.row(
                format!("debug-session-{id}"),
                format!("{} {name}", if *id == self.active_session { "●" } else { "○" }),
                DebugEvent::Session(*id),
                cx,
            ));
        }
        rows.push(header("Threads"));
        for (id, name) in &self.threads {
            rows.push(self.row(format!("debug-thread-{id}"), name.clone(), DebugEvent::Thread(*id), cx));
        }
        rows.push(header("Call stack"));
        for (id, name) in &self.frames {
            rows.push(self.row(format!("debug-frame-{id}"), name.clone(), DebugEvent::Frame(*id), cx));
        }
        rows.push(header("Variables (click to expand)"));
        for (i, v) in self.variables.iter().enumerate() {
            rows.push(self.row(
                format!("debug-variable-{i}"),
                format!("{}{}{} = {}", "  ".repeat(v.depth), if v.reference > 0 { "▸ " } else { "" }, v.name, v.value),
                DebugEvent::Expand(v.reference, i),
                cx,
            ));
        }
        rows.push(header("Watch expressions"));
        rows.push(self.row("debug-add-watch", "+ Add watch".into(), DebugEvent::AddWatch, cx));
        for (i, (name, value, _)) in self.watches.iter().enumerate() {
            rows.push(
                div()
                    .flex()
                    .child(self.row(format!("debug-watch-{i}"), format!("{name} = {value}"), DebugEvent::Watch(i), cx))
                    .child(self.row(format!("debug-remove-watch-{i}"), "×".into(), DebugEvent::RemoveWatch(i), cx))
                    .into_any_element(),
            );
        }
        rows.push(header("Breakpoints"));
        for (i, (path, line, label)) in self.breakpoints.iter().enumerate() {
            rows.push(
                div()
                    .flex()
                    .child(self.row(
                        format!("debug-bp-{i}"),
                        format!("{label} {}:{}", path.rsplit(['/', '\\']).next().unwrap_or(path), line + 1),
                        DebugEvent::ToggleBreakpoint(path.clone(), *line),
                        cx,
                    ))
                    .child(self.row(
                        format!("debug-bp-edit-{i}"),
                        "Edit".into(),
                        DebugEvent::EditBreakpoint(path.clone(), *line),
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        rows.push(header("Debug console"));
        for line in self.console.iter().rev().take(200).collect::<Vec<_>>().into_iter().rev() {
            rows.push(div().px(t.px(8.0)).font_family(crate::theme::CODE_FONT).child(line.clone()).into_any_element());
        }
        let controls = [
            ("Start", "debug.start"),
            ("Continue", "debug.continue"),
            ("Pause", "debug.pause"),
            ("Over", "debug.stepOver"),
            ("Into", "debug.stepInto"),
            ("Out", "debug.stepOut"),
            ("Restart", "debug.restart"),
            ("Stop", "debug.stop"),
            ("Clear console", "debug.clearConsole"),
        ];
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(t.fg)
            .text_size(t.px(12.0))
            .child(div().flex().flex_wrap().gap(t.px(4.0)).children(controls.into_iter().map(|(label, command)| {
                self.row(format!("debug-control-{command}"), label.into(), DebugEvent::Command(command), cx)
            })))
            .child(div().id("debug-body").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .child(Input::new(&self.input))
    }
}
