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
    #[cfg(target_os = "linux")]
    run_terminal_launcher();

    let started = std::time::Instant::now();
    let trace_startup = std::env::var_os("FORGE_STARTUP_TIMINGS").is_some();
    let stage = move |name: &str| {
        if trace_startup {
            eprintln!("forge_startup {name} {:.3}", started.elapsed().as_secs_f64() * 1000.);
        }
    };
    stage("main");
    forge_update::startup();
    stage("updater");
    let application = gpui_kit::application();
    stage("platform");
    application.run(move |cx: &mut App| {
        stage("callback");
        gpui_kit::init(cx);
        stage("kit");
        let fonts: Vec<Cow<'static, [u8]>> = theme::FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
        if let Err(err) = cx.text_system().add_fonts(fonts) {
            eprintln!("couldn't load Forge's fonts: {err}");
        }
        stage("fonts");
        app_state::init(cx);
        stage("state");
        editor::bind_keys(cx);
        // Closing the last window quits.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                forge_sys::process::stop_all_children();
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
        stage("workspace");
        cx.activate(true);
        stage("activation");
    });
}

#[cfg(target_os = "linux")]
fn run_terminal_launcher() {
    use std::os::unix::process::CommandExt;
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--forge-terminal-child")) {
        return;
    }
    let parent = args.next().and_then(|arg| arg.to_str()?.parse::<u32>().ok());
    let result = match (parent, args.next()) {
        (Some(parent), Some(program)) => forge_sys::process::set_parent_death_signal(parent)
            .and_then(|()| Err::<(), _>(std::process::Command::new(program).args(args).exec())),
        _ => Err(std::io::Error::other("Missing terminal parent or command")),
    };
    if let Err(error) = result {
        eprintln!("Could not start the terminal command: {error}");
        std::process::exit(126);
    }
}
