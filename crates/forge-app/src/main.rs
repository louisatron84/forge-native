//! Forge (native): the app.

// Parts of the views are wired up as the workspace is built (M2); remove when it is.
#![allow(dead_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_state;
mod document;
mod editor;
mod find_bar;
mod palette;
mod panels;
mod settings_view;
mod theme;
mod workspace;

use gpui_kit::App;
use std::borrow::Cow;
use std::path::PathBuf;

fn main() {
    forge_update::startup();
    gpui_kit::application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        let fonts: Vec<Cow<'static, [u8]>> = theme::FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
        if let Err(err) = cx.text_system().add_fonts(fonts) {
            eprintln!("couldn't load Forge's fonts: {err}");
        }
        app_state::init(cx);
        editor::bind_keys(cx);
        // Closing the last window quits.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                let state = app_state::state(cx);
                if let Err(error) = state.updater.install_on_exit(state.restart_after_update) {
                    eprintln!("Could not install update: {error}");
                }
                cx.quit();
            }
        })
        .detach();
        let folder = std::env::args().nth(1).map(PathBuf::from).filter(|p| p.is_dir());
        workspace::open_window(folder, cx);
        cx.activate(true);
    });
}
