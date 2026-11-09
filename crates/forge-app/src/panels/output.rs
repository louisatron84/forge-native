//! The Output panel: what a running Python program prints. stderr is red,
//! traceback locations are clickable, and while the program runs an input
//! box sends lines to it (so `input()` works). Only the last 20,000 lines are
//! kept. Replaces `src/renderer/outputView.ts`.

use crate::theme::{ForgeTheme, Theme, CODE_FONT};
use forge_core::output::{find_file_links, strip_ansi};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, prelude::FluentBuilder, px, uniform_list, AnyElement, App, AppContext, ClipboardItem, Context, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, ListHorizontalSizingBehavior, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, Window,
};
use std::sync::Arc;

/// Lines kept on screen; older ones are removed (with a note saying how many).
pub const MAX_LINES: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Out,
    Err,
    Meta,
    Ok,
    Fail,
    /// What you typed into the program.
    Echo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub kind: Kind,
    /// A traceback location: (file, line).
    pub link: Option<(String, usize)>,
}

/// The output as lines of coloured pieces.
#[derive(Clone, Debug, Default)]
pub struct OutputLog {
    pub lines: Vec<Vec<Seg>>,
    /// The last line hasn't ended yet (more text joins it).
    open: bool,
    /// Lines removed from the top (only the last MAX_LINES are kept).
    pub trimmed: usize,
}

impl OutputLog {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Add text; program output (`links`) has colour codes removed and traceback locations linked.
    pub fn push(&mut self, text: &str, kind: Kind, links: bool) {
        if text.is_empty() {
            return;
        }
        let text = if links { strip_ansi(text) } else { text.to_owned() };
        let text = text.replace("\r\n", "\n").replace('\t', "    ");
        let found = if links && text.contains("File \"") { find_file_links(&text) } else { Vec::new() };
        let mut pos = 0;
        for l in &found {
            if l.start > pos {
                self.push_plain(&text[pos..l.start], kind, None);
            }
            self.push_plain(&text[l.start..l.end], kind, Some((l.path.clone(), l.line)));
            pos = l.end;
        }
        if pos < text.len() {
            self.push_plain(&text[pos..], kind, None);
        }
        if self.lines.len() > MAX_LINES {
            let extra = self.lines.len() - MAX_LINES;
            self.lines.drain(..extra);
            self.trimmed += extra;
        }
    }

    fn push_plain(&mut self, text: &str, kind: Kind, link: Option<(String, usize)>) {
        for (i, piece) in text.split('\n').enumerate() {
            if i > 0 {
                // A line break: the next piece starts a new line.
                if !self.open {
                    self.lines.push(Vec::new());
                }
                self.open = false;
            }
            if piece.is_empty() {
                continue;
            }
            if !self.open {
                self.lines.push(Vec::new());
                self.open = true;
            }
            let line = self.lines.last_mut().expect("a line was just added");
            match line.last_mut() {
                Some(s) if s.kind == kind && s.link.is_none() && link.is_none() => s.text.push_str(piece),
                _ => line.push(Seg { text: piece.to_owned(), kind, link: link.clone() }),
            }
        }
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }
}

pub enum OutputEvent {
    OpenLink(String, usize),
    /// Text typed for the program (ends with a line break).
    SendInput(String),
    Run,
    Stop,
}

impl EventEmitter<OutputEvent> for OutputPanel {}

pub struct OutputPanel {
    focus: FocusHandle,
    log: OutputLog,
    running: bool,
    input: Entity<InputState>,
    scroll: UniformListScrollHandle,
    pub font_size: f32,
    _sub: Subscription,
}

impl Focusable for OutputPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl OutputPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type here and press Enter to send to the program"));
        let sub = cx.subscribe_in(&input, window, |this, input, e: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = e {
                let text = input.read(cx).value().to_string();
                input.update(cx, |s, cx| s.set_value("", window, cx));
                // Show what was typed, like a terminal would.
                this.push(&format!("{text}\n"), Kind::Echo, false, cx);
                cx.emit(OutputEvent::SendInput(format!("{text}\n")));
            }
        });
        Self {
            focus: cx.focus_handle(),
            log: OutputLog::default(),
            running: false,
            input,
            scroll: UniformListScrollHandle::new(),
            font_size: 13.0,
            _sub: sub,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.log.clear();
        cx.notify();
    }

    fn push(&mut self, text: &str, kind: Kind, links: bool, cx: &mut Context<Self>) {
        // Stick to the bottom only if you haven't scrolled up to read something.
        let follow = self.scroll.is_scrolled_to_end().unwrap_or(true) || self.log.lines.is_empty();
        self.log.push(text, kind, links);
        if follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    /// A run is about to start: clear old output and show what's running.
    pub fn begin(&mut self, what: &str, cx: &mut Context<Self>) {
        self.clear(cx);
        self.push(&format!("▶ {what}\n"), Kind::Meta, false, cx);
    }

    pub fn started(&mut self, python: &str, cx: &mut Context<Self>) {
        self.running = true;
        self.push(&format!("using {python}\n\n"), Kind::Meta, false, cx);
    }

    pub fn append(&mut self, stderr: bool, text: &str, cx: &mut Context<Self>) {
        self.push(text, if stderr { Kind::Err } else { Kind::Out }, true, cx);
    }

    pub fn error(&mut self, message: &str, cx: &mut Context<Self>) {
        self.push(&format!("{message}\n"), Kind::Err, false, cx);
    }

    pub fn finished(&mut self, code: Option<i32>, ms: u64, cx: &mut Context<Self>) {
        self.running = false;
        let secs = format!("{:.2}", ms as f64 / 1000.0);
        let (text, kind) = match code {
            None => (format!("\n■ Stopped after {secs}s\n"), Kind::Fail),
            Some(0) => (format!("\n✓ Exited with code 0 in {secs}s\n"), Kind::Ok),
            Some(c) => (format!("\n✗ Exited with code {c} in {secs}s\n"), Kind::Fail),
        };
        self.push(&text, kind, false, cx);
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        if self.running {
            self.input.update(cx, |s, cx| s.focus(window, cx));
        }
    }

    fn color(kind: Kind, t: &Theme) -> gpui_kit::Hsla {
        match kind {
            Kind::Out => t.fg,
            Kind::Err | Kind::Fail => t.err,
            Kind::Meta | Kind::Echo => t.fg_muted,
            Kind::Ok => t.ok,
        }
    }

    fn render_line(&self, i: usize, segs: &[Seg], t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(("out-line", i))
            .flex()
            .whitespace_nowrap()
            .h(px((self.font_size * 1.45).round()))
            .children(segs.iter().enumerate().map(|(j, s)| {
                let color = Self::color(s.kind, t);
                match &s.link {
                    None => div().text_color(color).child(SharedString::from(s.text.clone())).into_any_element(),
                    Some((path, line)) => {
                        let (path, line) = (path.clone(), *line);
                        div()
                            .id(SharedString::from(format!("out-link-{i}-{j}")))
                            .text_color(t.link)
                            .underline()
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |_, _, _, cx| cx.emit(OutputEvent::OpenLink(path.clone(), line))),
                            )
                            .child(SharedString::from(s.text.clone()))
                            .into_any_element()
                    }
                }
            }))
            .into_any_element()
    }
}

/// The buttons in the Output tab strip.
pub fn actions(panel: &Entity<OutputPanel>, t: &Theme) -> AnyElement {
    let button = |id: &'static str, label: &'static str, run: fn(&mut OutputPanel, &mut Context<OutputPanel>)| {
        let panel = panel.clone();
        div()
            .id(id)
            .px(px(6.0))
            .rounded(px(2.0))
            .text_size(px(12.0))
            .text_color(t.fg_muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_hover).text_color(t.fg))
            .on_click(move |_, _, cx| panel.update(cx, run))
            .child(label)
    };
    div()
        .flex()
        .gap(px(2.0))
        .child(button("out-run", "▶ Run", |_, cx| cx.emit(OutputEvent::Run)))
        .child(button("out-stop", "■ Stop", |_, cx| cx.emit(OutputEvent::Stop)))
        .child(button("out-copy", "Copy", |p, cx| cx.write_to_clipboard(ClipboardItem::new_string(p.log.text()))))
        .child(button("out-clear", "Clear", |p, cx| p.clear(cx)))
        .into_any_element()
}

impl Render for OutputPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let lines = Arc::new(self.log.lines.clone());
        let trimmed = self.log.trimmed;
        let list = uniform_list(
            "output-lines",
            lines.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = cx.global::<ForgeTheme>().0.clone();
                range.map(|i| this.render_line(i, &lines[i], &t, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .size_full();
        div()
            .id("output-panel")
            .key_context("Output")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .font_family(CODE_FONT)
            .text_size(px(self.font_size))
            .when(trimmed > 0, |d| {
                d.child(
                    div()
                        .px(px(10.0))
                        .py(px(2.0))
                        .text_color(t.fg_muted)
                        .text_size(px(11.0))
                        .child(format!("… {trimmed} earlier lines not shown (only the last {MAX_LINES} are kept)")),
                )
            })
            .child(div().flex_1().min_h_0().px(px(10.0)).py(px(6.0)).child(list))
            .when(self.running, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(10.0))
                        .py(px(4.0))
                        .border_t_1()
                        .border_color(t.border)
                        .child(div().text_color(t.fg_muted).child("Input ›"))
                        .child(div().flex_1().child(Input::new(&self.input).appearance(false).cleanable(false))),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(log: &OutputLog) -> Vec<String> {
        log.lines.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect()).collect()
    }

    #[test]
    fn splits_output_into_lines_joining_pieces_of_one_line() {
        let mut log = OutputLog::default();
        log.push("hel", Kind::Out, true);
        log.push("lo\nwor", Kind::Out, true);
        log.push("ld\n\nend", Kind::Out, true);
        assert_eq!(texts(&log), ["hello", "world", "", "end"]);
    }

    #[test]
    fn links_traceback_locations_and_strips_colours() {
        let mut log = OutputLog::default();
        log.push("Traceback:\n  File \"C:\\p\\a.py\", line 3, in f\n\x1b[31mValueError\x1b[0m\n", Kind::Err, true);
        assert_eq!(texts(&log), ["Traceback:", "  File \"C:\\p\\a.py\", line 3, in f", "ValueError"]);
        let link = log.lines[1].iter().find_map(|s| s.link.clone());
        assert_eq!(link, Some(("C:\\p\\a.py".into(), 3)));
    }

    #[test]
    fn keeps_streams_apart_and_only_the_last_lines() {
        let mut log = OutputLog::default();
        log.push("a", Kind::Out, true);
        log.push("b\n", Kind::Err, true);
        assert_eq!(log.lines[0].iter().map(|s| s.kind).collect::<Vec<_>>(), [Kind::Out, Kind::Err]);
        let many: String = (0..MAX_LINES + 5).map(|i| format!("{i}\n")).collect();
        log.push(&many, Kind::Out, false);
        assert_eq!(log.lines.len(), MAX_LINES);
        assert_eq!(log.trimmed, 6);
        assert_eq!(texts(&log).last().map(String::as_str), Some(&*format!("{}", MAX_LINES + 4)));
    }
}
