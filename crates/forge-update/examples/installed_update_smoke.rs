//! Console fixture for real installed updates on disposable Windows CI hosts.
//! Uses Forge's download/notes/apply-on-exit code without creating a GPUI window.
use forge_update::{State, Updater};
use std::{
    fs, io,
    path::PathBuf,
    time::{Duration, Instant},
};
use velopack::{sources::FileSource, UpdateManager};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    forge_update::startup();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, feed, data) = match args.as_slice() {
        [mode, feed, data, ..] => (mode, feed, data),
        _ => return Err(io::Error::other("expected apply|verify FEED DATA [ARMED EXIT]").into()),
    };
    let manager = UpdateManager::new(FileSource::new(PathBuf::from(feed)), None, None)?;
    if manager.get_is_portable() {
        return Err(io::Error::other("fixture must run from a real installation").into());
    }
    let version = manager.get_current_version_as_string();
    let updater = Updater::new(PathBuf::from(data));
    match (mode.as_str(), args.as_slice()) {
        ("apply", [_, _, _, armed, exit]) => {
            assert_eq!(version, "1.0.0");
            updater.check_for_validation(manager).map_err(io::Error::other)?;
            let State::Ready(release) = updater.state() else {
                return Err(io::Error::other("update did not reach Ready").into());
            };
            assert_eq!(release.version, "1.0.1");
            assert!(release.notes.contains("Installed update smoke fixture 1.0.1"));
            assert_eq!(updater.notes(), Some(release));
            updater.install_on_exit(false).map_err(io::Error::other)?;
            fs::write(armed, "armed")?;
            let deadline = Instant::now() + Duration::from_secs(90);
            while !std::path::Path::new(exit).exists() {
                if Instant::now() >= deadline {
                    return Err(io::Error::other("controller did not release fixture").into());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        ("verify", [_, _, _]) => {
            assert_eq!(version, "1.0.1");
            let notes = updater.notes().ok_or_else(|| io::Error::other("saved update notes missing"))?;
            assert_eq!(notes.version, version);
            assert!(notes.notes.contains("Installed update smoke fixture 1.0.1"));
            println!("installed_version={version}; saved_notes_version={}", notes.version);
        }
        _ => return Err(io::Error::other("invalid fixture arguments").into()),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Installed update smoke failed: {error}");
        std::process::exit(1);
    }
}
