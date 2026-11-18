//! Static Python analysis, replacing `src/main/projectScan.ts`.
//! The unchanged AST scanner reads source, including unsaved editor overrides;
//! it never imports or executes project code. Call these functions off-thread.

use forge_core::project_graph::{RawFile, RawScan};
use std::{collections::BTreeMap, path::Path, time::Duration};

pub const SCAN_SCRIPT: &str = include_str!("../../../python/projectScan.py");
pub const DATAFLOW_SCRIPT: &str = include_str!("../../../python/dataflow.py");

/// Reads source from stdin, including unsaved edits; never imports project code.
pub fn run_dataflow(
    python: &str,
    root: &Path,
    source: &str,
    func: &str,
    timeout: Duration,
) -> Result<forge_core::dataflow::RawFlow, Error> {
    let output = forge_sys::process::run_captured(
        python,
        &[
            "-I".into(),
            "-S".into(),
            "-B".into(),
            "-X".into(),
            "utf8".into(),
            "-c".into(),
            DATAFLOW_SCRIPT.into(),
            func.into(),
        ],
        root,
        &[("PYTHONIOENCODING", "utf-8"), ("PYTHONDONTWRITEBYTECODE", "1")],
        source,
        timeout,
    );
    if output.failed {
        return Err(Error::Failed(format!("Couldn't start {python}")));
    }
    if output.code.is_none() {
        return Err(Error::Timeout);
    }
    if !output.ok() {
        return Err(Error::Failed(output.stderr.chars().rev().take(2000).collect::<String>().chars().rev().collect()));
    }
    Ok(serde_json::from_str(&output.stdout)?)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Python was not found. Install it from python.org, or select a Python interpreter.")]
    NoPython,
    #[error("The analysis failed: {0}")]
    Failed(String),
    #[error("The analysis timed out.")]
    Timeout,
    #[error("The analysis path must be inside the project: {0}")]
    InvalidPath(String),
    #[error("Couldn't prepare the analysis: {0}")]
    Io(#[from] std::io::Error),
    #[error("Couldn't read the analysis: {0}")]
    Json(#[from] serde_json::Error),
}

fn valid_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains([':', '\0'])
        && !path.split(['/', '\\']).any(|p| p == "..")
}

fn run_script(
    python: &str,
    root: &Path,
    overrides: &BTreeMap<String, String>,
    only: Option<&[String]>,
    timeout: Duration,
) -> Result<String, Error> {
    for path in overrides.keys().chain(only.into_iter().flatten()) {
        if !valid_relative(path) {
            return Err(Error::InvalidPath(path.clone()));
        }
    }
    // These helpers only need the standard library. Ignore project/PYTHONPATH
    // imports and site startup hooks; an AST scan must never execute project code.
    let mut args = vec![
        "-I".into(),
        "-S".into(),
        "-B".into(),
        "-X".into(),
        "utf8".into(),
        "-".into(),
        root.to_string_lossy().into_owned(),
    ];
    let mut temporary = None;
    if !overrides.is_empty() || only.is_some() {
        // A private file, removed on success, failure, timeout and bad JSON.
        let file = forge_sys::file_write::scratch(serde_json::to_vec(overrides)?)?;
        args.extend(["--overrides".into(), file.path().to_string_lossy().into_owned()]);
        temporary = Some(file);
    }
    if let Some(only) = only {
        args.push("--only".into());
        args.extend_from_slice(only);
    }
    let output = forge_sys::process::run_captured(
        python,
        &args,
        root,
        &[("PYTHONIOENCODING", "utf-8"), ("PYTHONDONTWRITEBYTECODE", "1")],
        SCAN_SCRIPT,
        timeout,
    );
    drop(temporary);
    if output.failed {
        return Err(Error::Failed(format!("Couldn't start {python}")));
    }
    if output.code.is_none() {
        return Err(Error::Timeout);
    }
    if !output.ok() {
        return Err(Error::Failed(if output.stderr.trim().is_empty() {
            format!("exit code {:?}", output.code)
        } else {
            output.stderr.chars().rev().take(4000).collect::<String>().chars().rev().collect()
        }));
    }
    Ok(output.stdout)
}

pub fn run_scan(
    python: &str,
    root: &Path,
    overrides: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<RawScan, Error> {
    Ok(serde_json::from_str(&run_script(python, root, overrides, None, timeout)?)?)
}
pub fn run_scan_files(
    python: &str,
    root: &Path,
    files: &[String],
    overrides: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<Vec<RawFile>, Error> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&run_script(python, root, overrides, Some(files), timeout)?)?)
}
pub fn scan_project(
    root: &Path,
    configured: Option<&str>,
    overrides: &BTreeMap<String, String>,
) -> Result<RawScan, Error> {
    let python = forge_run::python::find_python(root, configured).ok_or(Error::NoPython)?;
    run_scan(&python, root, overrides, Duration::from_secs(120))
}
pub fn scan_project_files(
    root: &Path,
    configured: Option<&str>,
    files: &[String],
    overrides: &BTreeMap<String, String>,
) -> Result<Vec<RawFile>, Error> {
    let python = forge_run::python::find_python(root, configured).ok_or(Error::NoPython)?;
    run_scan_files(&python, root, files, overrides, Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_outside_the_project_before_starting_a_process() {
        for path in ["../secret.py", "C:\\secret.py", "/secret.py", "sub/../../secret.py"] {
            let overrides = BTreeMap::from([(path.into(), String::new())]);
            assert!(matches!(
                run_scan("does-not-exist", Path::new("."), &overrides, Duration::ZERO),
                Err(Error::InvalidPath(_))
            ));
        }
    }
}
