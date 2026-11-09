//! Background native updates and release notes. Replaces the updates UI in main.ts.
use super::Workspace;
use crate::app_state::{self, AppState};
use crate::palette::Choice;
use forge_update::State;
use gpui_kit::{Context, Window};
use std::time::Duration;

impl Workspace {
    pub(crate) fn start_updates(&mut self, cx: &mut Context<Self>) {
        let updater = app_state::state(cx).updater.clone();
        self.update_poll = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            loop {
                let automatic = this.read_with(cx, |_, cx| app_state::settings(cx).check_for_updates).unwrap_or(false);
                if automatic {
                    let service = updater.clone();
                    cx.background_executor().spawn(async move { service.check(true) }).await;
                    if this.update(cx, |ws, cx| ws.update_status(cx)).is_err() {
                        break;
                    }
                }
                cx.background_executor().timer(Duration::from_secs(6 * 60 * 60)).await;
            }
        }));
        let notes = app_state::state(cx).updater.whats_new();
        if let Some(notes) = notes {
            self.release_notes = Some(notes);
        }
    }

    pub(crate) fn update_status(&mut self, cx: &mut Context<Self>) {
        let label = app_state::state(cx).updater.state().label();
        self.settings_view.update(cx, |s, cx| {
            s.update_info = label;
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn check_updates(&mut self, download: bool, window: &mut Window, cx: &mut Context<Self>) {
        let updater = app_state::state(cx).updater.clone();
        self.status_note(if download { "Downloading update…" } else { "Checking for updates…" }, cx);
        cx.spawn_in(window, async move |this, cx| {
            let state = cx.background_executor().spawn(async move { updater.check(download) }).await;
            this.update_in(cx, |ws, window, cx| {
                ws.update_status(cx);
                let mut choices =
                    vec![Choice::new("Release notes", |ws, window, cx| ws.show_release_notes(window, cx))];
                match state {
                    State::Ready(_) => choices.insert(
                        0,
                        Choice::new("Restart now", |ws, window, cx| {
                            // The normal close path handles unsaved files. Other open windows
                            // still have to close before the process can be updated.
                            cx.global_mut::<AppState>().restart_after_update = true;
                            if ws.should_close(window, cx) {
                                window.remove_window();
                            }
                        }),
                    ),
                    State::Available(_) => {
                        if forge_update::can_self_update() {
                            choices.insert(
                                0,
                                Choice::new("Download update", |ws, window, cx| ws.check_updates(true, window, cx)),
                            );
                        }
                        choices.push(Choice::new("Open downloads", |_, _, cx| {
                            cx.open_url(&format!("{}/releases/latest", forge_update::REPOSITORY))
                        }));
                    }
                    _ => {}
                }
                ws.choose(state.label(), choices, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn show_release_notes(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let service = app_state::state(cx).updater.clone();
        let release = service.notes().or_else(|| match service.state() {
            State::Available(r) | State::Ready(r) | State::Downloading(r) => Some(r),
            _ => None,
        });
        let (title, text) =
            release.map(|r| (format!("What's new in Forge {}", r.version), r.notes)).unwrap_or_else(|| {
                (format!("Forge {}", forge_update::CURRENT), "No native release notes are available yet.".into())
            });
        self.git.diff = None;
        self.git_inspect.update(cx, |p, cx| p.text(title, text.lines().map(str::to_owned).collect(), cx));
        self.reveal_panel("editor", cx);
    }
}
