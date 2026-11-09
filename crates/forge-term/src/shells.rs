//! The shells installed on this machine (for new terminals and the Settings
//! dialog). Replaces `findShells` / `defaultShell` in `src/main/terminals.ts`.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellProfile {
    /// Stored in settings ("powershell", "gitbash"...).
    pub id: String,
    pub name: String,
    pub file: String,
    pub args: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Mac,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        }
    }
}

fn win_join(parts: &[&str]) -> String {
    parts.iter().map(|p| p.trim_end_matches('\\')).collect::<Vec<_>>().join("\\")
}

/// The shell used when none is chosen.
pub fn default_shell(platform: Platform, env: &HashMap<String, String>) -> ShellProfile {
    match platform {
        Platform::Windows => ShellProfile {
            id: "powershell".into(),
            name: "PowerShell".into(),
            file: "powershell.exe".into(),
            args: vec!["-NoLogo".into()],
        },
        _ => {
            let file = env.get("SHELL").cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/bash".into());
            let name = file.rsplit('/').next().unwrap_or(&file).to_owned();
            ShellProfile { id: name.clone(), name, file, args: vec!["-l".into()] }
        }
    }
}

/// Shells installed on this machine, the default first. `exists` checks a
/// path (passed in so this can be tested anywhere).
pub fn find_shells(
    platform: Platform,
    env: &HashMap<String, String>,
    exists: impl Fn(&str) -> bool,
) -> Vec<ShellProfile> {
    let mut out: Vec<ShellProfile> = Vec::new();
    fn add(out: &mut Vec<ShellProfile>, id: &str, name: &str, file: String, args: &[&str]) {
        if !out.iter().any(|s| s.id == id) {
            out.push(ShellProfile {
                id: id.into(),
                name: name.into(),
                file,
                args: args.iter().map(|a| (*a).to_owned()).collect(),
            });
        }
    }
    let get = |k: &str| env.get(k).filter(|v| !v.is_empty()).cloned();
    if platform == Platform::Windows {
        let sys_root = get("SystemRoot").or_else(|| get("windir")).unwrap_or_else(|| "C:\\Windows".into());
        let program_files: Vec<String> =
            ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"].iter().filter_map(|k| get(k)).collect();
        add(&mut out, "powershell", "PowerShell", "powershell.exe".into(), &["-NoLogo"]);
        for pf in &program_files {
            let pwsh = win_join(&[pf, "PowerShell", "7", "pwsh.exe"]);
            if exists(&pwsh) {
                add(&mut out, "pwsh", "PowerShell 7", pwsh, &["-NoLogo"]);
            }
        }
        add(&mut out, "cmd", "Command Prompt", win_join(&[&sys_root, "System32", "cmd.exe"]), &[]);
        for pf in &program_files {
            let bash = win_join(&[pf, "Git", "bin", "bash.exe"]);
            if exists(&bash) {
                add(&mut out, "gitbash", "Git Bash", bash, &["--login", "-i"]);
            }
        }
        let wsl = win_join(&[&sys_root, "System32", "wsl.exe"]);
        if exists(&wsl) {
            add(&mut out, "wsl", "WSL", wsl, &[]);
        }
        return out;
    }
    if let Some(shell) = get("SHELL") {
        let name = shell.rsplit('/').next().unwrap_or(&shell).to_owned();
        add(&mut out, &name, &name, shell.clone(), &["-l"]);
    }
    let known: [(&str, &[&str]); 3] = [
        ("bash", &["/bin/bash", "/usr/bin/bash", "/opt/homebrew/bin/bash"]),
        ("zsh", &["/bin/zsh", "/usr/bin/zsh"]),
        ("fish", &["/usr/bin/fish", "/opt/homebrew/bin/fish", "/usr/local/bin/fish"]),
    ];
    for (name, paths) in known {
        if let Some(found) = paths.iter().find(|p| exists(p)) {
            add(&mut out, name, name, (*found).to_owned(), &["-l"]);
        }
    }
    if out.is_empty() {
        add(&mut out, "sh", "sh", "/bin/sh".into(), &[]);
    }
    out
}

/// The shells installed here (from the real environment and disk).
pub fn installed_shells() -> Vec<ShellProfile> {
    let env: HashMap<String, String> = std::env::vars().collect();
    find_shells(Platform::current(), &env, |p| std::path::Path::new(p).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn windows_powershell_first_then_whatever_else_is_installed() {
        let installed = ["C:\\Program Files\\PowerShell\\7\\pwsh.exe", "C:\\Program Files\\Git\\bin\\bash.exe"];
        let shells = find_shells(
            Platform::Windows,
            &env(&[("SystemRoot", "C:\\Windows"), ("ProgramFiles", "C:\\Program Files")]),
            |p| installed.contains(&p),
        );
        assert_eq!(
            shells.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["PowerShell", "PowerShell 7", "Command Prompt", "Git Bash"]
        );
        assert_eq!(
            shells.iter().find(|s| s.id == "cmd").map(|s| s.file.as_str()),
            Some("C:\\Windows\\System32\\cmd.exe")
        );
        assert_eq!(
            shells.iter().find(|s| s.id == "gitbash").map(|s| s.args.clone()),
            Some(vec!["--login".to_owned(), "-i".to_owned()])
        );
    }

    #[test]
    fn windows_wsl_when_present() {
        let shells = find_shells(Platform::Windows, &env(&[("SystemRoot", "C:\\Windows")]), |p| {
            p == "C:\\Windows\\System32\\wsl.exe"
        });
        assert_eq!(shells.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["powershell", "cmd", "wsl"]);
    }

    #[test]
    fn mac_and_linux_the_login_shell_first_then_others() {
        let installed = ["/bin/bash", "/bin/zsh"];
        let shells = find_shells(Platform::Mac, &env(&[("SHELL", "/bin/zsh")]), |p| installed.contains(&p));
        assert_eq!(shells.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["zsh", "bash"]);
    }

    #[test]
    fn falls_back_to_sh() {
        let shells = find_shells(Platform::Linux, &env(&[]), |_| false);
        assert_eq!(shells.iter().map(|s| s.file.as_str()).collect::<Vec<_>>(), ["/bin/sh"]);
    }

    #[test]
    fn default_shell_follows_the_platform() {
        assert_eq!(default_shell(Platform::Windows, &env(&[])).file, "powershell.exe");
        assert_eq!(default_shell(Platform::Linux, &env(&[("SHELL", "/usr/bin/fish")])).name, "fish");
    }
}
