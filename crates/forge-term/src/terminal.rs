//! A running terminal: a shell (or a program) in a pseudo-terminal (ConPTY on
//! Windows), read on its own thread into the screen model. Replaces node-pty
//! and xterm.js's parser (`src/main/terminals.ts`).

use crate::screen::{self, Palette, Size, Snapshot};
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{ClearMode, Handler};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(windows)]
type TerminalProcess = forge_sys::process::WindowsJob;

#[cfg(unix)]
struct TerminalProcess {
    tty: std::fs::File,
    session: libc::pid_t,
}

#[cfg(unix)]
impl Drop for TerminalProcess {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        let tty = self.tty.as_raw_fd();
        // Only signal groups still attached to this specific controlling terminal.
        unsafe {
            if self.session > 1 && libc::tcgetsid(tty) == self.session {
                let foreground = libc::tcgetpgrp(tty);
                if foreground > 1 && foreground != self.session {
                    libc::killpg(foreground, libc::SIGKILL);
                }
                libc::killpg(self.session, libc::SIGKILL);
            }
        }
    }
}

/// What a terminal tells its view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermEvent {
    /// New output to draw.
    Wakeup,
    Title(String),
    ResetTitle,
    Bell,
    /// The program asked to store text in the clipboard (OSC 52).
    Clipboard(String),
    /// The shell or program ended (exit code when known).
    Exited(Option<i32>),
}

/// Passes the model's events on to the view, and answers the program's queries.
#[derive(Clone)]
pub struct Listener {
    events: Arc<dyn Fn(TermEvent) + Send + Sync>,
    /// Replies the program asked for (cursor position, colours...) go back to the pty.
    reply: Arc<FairMutex<Option<EventLoopSender>>>,
    process: Arc<FairMutex<Option<TerminalProcess>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let e = match event {
            Event::Wakeup => TermEvent::Wakeup,
            Event::Title(t) => TermEvent::Title(t),
            Event::ResetTitle => TermEvent::ResetTitle,
            Event::Bell => TermEvent::Bell,
            Event::ClipboardStore(_, text) => TermEvent::Clipboard(text),
            Event::ChildExit(status) => {
                self.process.lock().take();
                TermEvent::Exited(status.code())
            }
            Event::Exit => {
                self.process.lock().take();
                TermEvent::Exited(None)
            }
            Event::PtyWrite(text) => {
                if let Some(s) = self.reply.lock().as_ref() {
                    let _ = s.send(Msg::Input(Cow::Owned(text.into_bytes())));
                }
                return;
            }
            _ => return,
        };
        (self.events)(e);
    }
}

/// How to start a terminal.
#[derive(Clone, Debug, Default)]
pub struct StartOptions {
    /// The program and its arguments; None = the system's default shell.
    pub program: Option<(String, Vec<String>)>,
    pub cwd: Option<PathBuf>,
    /// Added to the environment.
    pub env: HashMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    /// The Forge executable that configures parent-death before exec on Linux.
    #[cfg(target_os = "linux")]
    pub launcher: Option<PathBuf>,
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    process: Arc<FairMutex<Option<TerminalProcess>>>,
    size: Size,
}

impl Terminal {
    /// Start a shell or program; `on_event` hears about its output, title and exit
    /// (on the terminal's reader thread).
    pub fn spawn(opts: StartOptions, on_event: impl Fn(TermEvent) + Send + Sync + 'static) -> io::Result<Terminal> {
        let reply = Arc::new(FairMutex::new(None));
        let process = Arc::new(FairMutex::new(None));
        let listener = Listener { events: Arc::new(on_event), reply: reply.clone(), process: process.clone() };
        let cols = opts.cols.max(2);
        let rows = opts.rows.max(2);
        let size = Size { cols: cols as usize, lines: rows as usize };
        let config = Config { scrolling_history: 5000, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let mut env = opts.env.clone();
        env.entry("TERM".into()).or_insert_with(|| "xterm-256color".into());
        env.entry("COLORTERM".into()).or_insert_with(|| "truecolor".into());
        env.entry("TERM_PROGRAM".into()).or_insert_with(|| "Forge".into());
        let program = opts.program;
        #[cfg(target_os = "linux")]
        let program = if let Some(launcher) = opts.launcher {
            let launcher = launcher
                .into_os_string()
                .into_string()
                .map_err(|_| io::Error::other("Forge's executable path is not valid UTF-8"))?;
            let (shell, args) = program.unwrap_or_else(|| {
                (std::env::var("SHELL").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "/bin/sh".into()), Vec::new())
            });
            let mut command = vec!["--forge-terminal-child".into(), std::process::id().to_string(), shell];
            command.extend(args);
            Some((launcher, command))
        } else {
            program
        };
        let options = tty::Options {
            shell: program.map(|(p, a)| tty::Shell::new(p, a)),
            working_directory: opts.cwd.filter(|d| d.is_dir()),
            drain_on_exit: true,
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let window = WindowSize { num_lines: rows, num_cols: cols, cell_width: 8, cell_height: 16 };
        let pty = tty::new(&options, window, 0)?;
        #[cfg(windows)]
        {
            use std::os::windows::io::BorrowedHandle;
            let handle = pty.child_watcher().raw_handle();
            if !handle.is_null() {
                // SAFETY: the PTY keeps the handle open while borrowed.
                let child = unsafe { BorrowedHandle::borrow_raw(handle) };
                *process.lock() = TerminalProcess::for_terminal(child)?;
            }
        }
        #[cfg(unix)]
        {
            *process.lock() =
                Some(TerminalProcess { tty: pty.file().try_clone()?, session: pty.child().id() as libc::pid_t });
        }
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        *reply.lock() = Some(sender.clone());
        event_loop.spawn();
        Ok(Terminal { term, sender, process, size })
    }

    /// Send keys or text to the program.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        // Typing jumps back to the bottom of the output.
        {
            let mut t = self.term.lock();
            let offset = t.grid().display_offset() as i32;
            if offset > 0 {
                screen::scroll(&mut t, -offset);
            }
        }
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// The view changed size: re-flow the screen and tell the program.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        let (cols, rows) = (cols.max(2), rows.max(2));
        let size = Size { cols: cols as usize, lines: rows as usize };
        if size == self.size {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        let _ = self.sender.send(Msg::Resize(WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height }));
    }

    /// End the program (the terminal stays readable).
    pub fn kill(&self) {
        self.process.lock().take();
        let _ = self.sender.send(Msg::Shutdown);
    }

    pub fn snapshot(&self, palette: &Palette) -> Snapshot {
        screen::snapshot(&self.term.lock(), palette)
    }

    /// Scroll back (positive) or forward (negative) through the output.
    pub fn scroll(&self, lines: i32) {
        screen::scroll(&mut self.term.lock(), lines);
    }

    pub fn start_selection(&self, line: usize, col: usize, right_half: bool, clicks: usize) {
        screen::start_selection(&mut self.term.lock(), line, col, right_half, clicks);
    }

    pub fn update_selection(&self, line: usize, col: usize, right_half: bool) {
        screen::update_selection(&mut self.term.lock(), line, col, right_half);
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    /// Clear the screen and the history (the shell draws its prompt again).
    pub fn clear(&self) {
        {
            let mut t = self.term.lock();
            t.clear_screen(ClearMode::Saved);
            t.clear_screen(ClearMode::All);
            t.goto(0, 0);
        }
        let _ = self.sender.send(Msg::Input(Cow::Borrowed(b"\r")));
    }

    /// Show a note in the terminal itself (e.g. "[shell exited...]"), as if the program printed it.
    pub fn show_message(&self, text: &str) {
        let mut t = self.term.lock();
        let mut p: alacritty_terminal::vte::ansi::Processor = alacritty_terminal::vte::ansi::Processor::new();
        p.advance(&mut *t, text.as_bytes());
    }

    /// Everything in the terminal as text (the history and the screen).
    pub fn text(&self) -> String {
        let t = self.term.lock();
        let grid = t.grid();
        let top = -(grid.total_lines().saturating_sub(grid.screen_lines()) as i32);
        let bottom = grid.screen_lines() as i32 - 1;
        let mut out = Vec::new();
        for line in top..=bottom {
            let row = &grid[alacritty_terminal::index::Line(line)];
            let text: String = (0..grid.columns()).map(|c| row[alacritty_terminal::index::Column(c)].c).collect();
            out.push(text.trim_end().to_owned());
        }
        while out.last().is_some_and(String::is_empty) {
            out.pop();
        }
        out.join("\n")
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn runs_a_program_and_shows_its_output_then_its_exit() {
        let program = if cfg!(windows) {
            ("cmd.exe".to_owned(), vec!["/c".to_owned(), "echo hello-forge".to_owned()])
        } else {
            ("/bin/sh".to_owned(), vec!["-c".to_owned(), "echo hello-forge".to_owned()])
        };
        let (tx, events) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let term = Terminal::spawn(
            StartOptions { program: Some(program), cols: 80, rows: 10, ..Default::default() },
            move |e| {
                let _ = tx.lock().map(|t| t.send(e));
            },
        )
        .expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut exited = false;
        while Instant::now() < deadline {
            if let Ok(TermEvent::Exited(_)) = events.recv_timeout(Duration::from_millis(200)) {
                exited = true;
                break;
            }
        }
        assert!(exited, "the program never finished");
        assert!(term.text().contains("hello-forge"), "output was: {:?}", term.text());
    }
}
