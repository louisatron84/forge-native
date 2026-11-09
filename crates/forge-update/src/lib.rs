//! Native updates, background downloads and release notes.
//! Replaces `src/main/updates.ts` and `src/shared/releaseNotes.ts`.
//! Packaged builds use Velopack; source/portable copies offer the release page.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use velopack::{sources::GithubSource, UpdateCheck, UpdateInfo, UpdateManager};

pub const REPOSITORY: &str = match option_env!("FORGE_RELEASE_REPOSITORY") {
    Some(url) => url,
    None => "https://github.com/louisatron84/forge-releases",
};
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Checking,
    Current,
    Available(Release),
    Downloading(Release),
    Ready(Release),
    Error(String),
}

impl State {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => format!("Forge {CURRENT}"),
            Self::Checking => "Checking for updates…".into(),
            Self::Current => "Forge is up to date.".into(),
            Self::Available(r) => format!("Forge {} is available. Open downloads.", r.version),
            Self::Downloading(r) => format!("Downloading Forge {}…", r.version),
            Self::Ready(r) => format!("Forge {} is ready. Restart now, or install when Forge closes.", r.version),
            Self::Error(e) => format!("Update check failed: {e}"),
        }
    }
}

pub struct Updater {
    state: Mutex<State>,
    busy: AtomicBool,
    pending: Mutex<Option<(UpdateManager, UpdateInfo)>>,
    dir: PathBuf,
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    match (
        semver::Version::parse(current.trim_start_matches(['v', 'V'])),
        semver::Version::parse(latest.trim_start_matches(['v', 'V'])),
    ) {
        (Ok(a), Ok(b)) => b > a,
        _ => false,
    }
}

pub fn parse_release(value: &serde_json::Value) -> Option<Release> {
    if value["draft"] == true || value["prerelease"] == true {
        return None;
    }
    Some(Release {
        version: value["tag_name"].as_str()?.trim_start_matches(['v', 'V']).into(),
        notes: value["body"].as_str().unwrap_or("").chars().take(8000).collect(),
        url: format!("{REPOSITORY}/releases/latest"),
    })
}

impl Updater {
    pub fn new(dir: PathBuf) -> Self {
        Self { state: Mutex::new(State::Idle), busy: AtomicBool::new(false), pending: Mutex::new(None), dir }
    }
    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_else(|_| State::Error("Updater unavailable".into()))
    }
    fn set(&self, state: State) {
        if let Ok(mut s) = self.state.lock() {
            *s = state;
        }
    }

    /// Blocking. Network/downloads must run on a worker, never on the UI thread.
    pub fn check(&self, automatic: bool) -> State {
        if matches!(self.state(), State::Ready(_) | State::Downloading(_)) || self.busy.swap(true, Ordering::SeqCst) {
            return self.state();
        }
        self.set(State::Checking);
        let result = self.check_inner(automatic);
        if let Err(error) = result {
            self.set(State::Error(error));
        }
        self.busy.store(false, Ordering::SeqCst);
        self.state()
    }

    fn check_inner(&self, automatic: bool) -> Result<(), String> {
        let manager = UpdateManager::new(GithubSource::new(REPOSITORY, None, false), None, None);
        if let Ok(manager) = manager {
            match manager.check_for_updates().map_err(|e| e.to_string())? {
                UpdateCheck::UpdateAvailable(info) => {
                    let release = Release {
                        version: info.TargetFullRelease.Version.clone(),
                        notes: info.TargetFullRelease.NotesMarkdown.clone(),
                        url: format!("{REPOSITORY}/releases/latest"),
                    };
                    if !automatic
                        || (manager.get_is_portable()
                            && !(cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some()))
                    {
                        self.set(State::Available(release));
                        return Ok(());
                    }
                    self.set(State::Downloading(release.clone()));
                    manager.download_updates(&info, None).map_err(|e| e.to_string())?;
                    self.save_notes(&release)?;
                    *self.pending.lock().map_err(|_| "Updater unavailable")? = Some((manager, *info));
                    self.set(State::Ready(release));
                }
                _ => self.set(State::Current),
            }
        } else {
            let repo = REPOSITORY.strip_prefix("https://github.com/").ok_or("Invalid release repository")?;
            let url = format!("https://api.github.com/repos/{repo}/releases/latest");
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(20)))
                .build()
                .new_agent();
            let response = agent
                .get(&url)
                .header("User-Agent", "Forge-update-check")
                .header("Accept", "application/vnd.github+json")
                .call();
            match response {
                Ok(mut response) => {
                    let value: serde_json::Value = response.body_mut().read_json().map_err(|e| e.to_string())?;
                    match parse_release(&value).filter(|r| is_newer(CURRENT, &r.version)) {
                        Some(release) => self.set(State::Available(release)),
                        None => self.set(State::Current),
                    }
                }
                Err(ureq::Error::StatusCode(404)) => {
                    self.set(State::Error("No native release has been published yet.".into()))
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }

    /// Arm the installer only after every window has accepted closing; it waits
    /// for this process to exit, with no work scheduled while the user is coding.
    pub fn install_on_exit(&self, restart: bool) -> Result<(), String> {
        let pending = self.pending.lock().map_err(|_| "Updater unavailable")?;
        if let Some((manager, info)) = pending.as_ref() {
            manager
                .wait_exit_then_apply_updates(info, true, restart, Vec::<String>::new())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn save_notes(&self, release: &Release) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("update-notes.json"), serde_json::to_vec(release).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    pub fn notes(&self) -> Option<Release> {
        serde_json::from_slice(&std::fs::read(self.dir.join("update-notes.json")).ok()?).ok()
    }
    pub fn whats_new(&self) -> Option<Release> {
        let path = self.dir.join("last-run-version.txt");
        let previous = std::fs::read_to_string(&path).ok();
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(path, CURRENT);
        if previous.as_deref().is_some_and(|v| is_newer(v.trim(), CURRENT)) {
            self.notes().filter(|r| r.version == CURRENT)
        } else {
            None
        }
    }
}

pub fn startup() {
    velopack::VelopackApp::build().run();
}

pub fn can_self_update() -> bool {
    UpdateManager::new(GithubSource::new(REPOSITORY, None, false), None, None)
        .is_ok_and(|m| !m.get_is_portable() || (cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some()))
}

pub fn notes_from_disk(dir: &Path) -> Option<Release> {
    Updater::new(dir.to_path_buf()).notes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_versions_and_rejects_invalid_versions() {
        assert!(is_newer("0.50.0", "1.0.0"));
        assert!(is_newer("1.0.0-dev", "1.0.0"));
        assert!(!is_newer("1.0.0", "0.50.0"));
        assert!(!is_newer("bad", "1.0.0"));
    }
    #[test]
    fn release_parser_ignores_drafts_and_prereleases_and_caps_notes() {
        let release = serde_json::json!({"tag_name":"v1.2.0","body":"x".repeat(9000)});
        assert_eq!(parse_release(&release).unwrap().notes.len(), 8000);
        assert!(parse_release(&serde_json::json!({"tag_name":"v1.2.0","prerelease":true})).is_none());
        assert!(parse_release(&serde_json::json!({"tag_name":12})).is_none());
    }
    #[test]
    fn notes_are_saved_and_whats_new_is_shown_once_after_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let updater = Updater::new(dir.path().into());
        let release = Release { version: CURRENT.into(), notes: "New Forge".into(), url: String::new() };
        updater.save_notes(&release).unwrap();
        assert!(updater.whats_new().is_none());
        std::fs::write(dir.path().join("last-run-version.txt"), "0.50.0").unwrap();
        assert_eq!(updater.whats_new(), Some(release));
        assert!(updater.whats_new().is_none());
    }
}
