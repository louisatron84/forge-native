//! Project scaffolding and user templates. Never follow captured symlinks or copy secrets/caches.
use forge_core::templates::*;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
pub fn create_project(parent: &Path, name: &str, files: &[TemplateFile]) -> io::Result<PathBuf> {
    if let Some(e) = project_name_problem(name) {
        return Err(invalid(e));
    }
    let mut paths = BTreeSet::new();
    for f in files {
        if !safe_template_path(&f.path) || !paths.insert(f.path.replace('\\', "/").to_lowercase()) {
            return Err(invalid(format!("The template has a file Forge won't write: {}", f.path)));
        }
    }
    let target = parent.join(name.trim());
    if target.exists() {
        return Err(invalid("A folder with that name already exists."));
    }
    fs::create_dir(&target)?;
    for f in files {
        let path = target.join(f.path.replace('\\', "/"));
        fs::create_dir_all(path.parent().unwrap())?;
        fs::OpenOptions::new().create_new(true).write(true).open(path)?.write_all(f.content.as_bytes())?;
    }
    Ok(target)
}
fn text_files(root: &Path) -> io::Result<Vec<TemplateFile>> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let mut entries = fs::read_dir(&dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        let mut folders = Vec::new();
        for e in entries {
            if out.len() >= 300 {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if [
                ".git",
                ".hg",
                ".svn",
                ".venv",
                "venv",
                "env",
                "__pycache__",
                "node_modules",
                ".pytest_cache",
                ".mypy_cache",
                ".ruff_cache",
                ".tox",
                "build",
                "dist",
                "target",
                ".idea",
                ".vscode",
                ".env",
                ".DS_Store",
            ]
            .contains(&name.as_str())
                || name.starts_with(".env.")
                || name.ends_with(".egg-info")
            {
                continue;
            }
            let kind = e.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            let path = e.path();
            if kind.is_dir() {
                if !path.join("pyvenv.cfg").exists() {
                    folders.push(path);
                }
            } else if kind.is_file() && e.metadata()?.len() <= 512 * 1024 {
                let bytes = fs::read(&path)?;
                if bytes.contains(&0) {
                    continue;
                }
                if let Ok(content) = String::from_utf8(bytes) {
                    let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                    if safe_template_path(&rel) {
                        out.push(TemplateFile { path: rel, content });
                    }
                }
            }
        }
        todo.extend(folders.into_iter().rev());
        if out.len() >= 300 {
            break;
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
fn swap_module(text: &str, module: &str) -> String {
    if module.len() < 3 {
        return text.into();
    }
    let mut out = String::new();
    let mut at = 0;
    for (i, _) in text.match_indices(module) {
        let end = i + module.len();
        let left = text[..i].chars().next_back();
        let right = text[end..].chars().next();
        if left.is_some_and(|c| c.is_ascii_alphanumeric()) || right.is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        out.push_str(&text[at..i]);
        out.push_str("{{module}}");
        at = end;
    }
    out.push_str(&text[at..]);
    out
}
pub fn save_template(root: &Path, store: &Path, name: &str, detail: &str) -> io::Result<String> {
    if name.trim().is_empty() {
        return Err(invalid("Type a name for the template."));
    }
    let project = root.file_name().unwrap_or_default().to_string_lossy();
    let module = module_name(&project);
    let swap = |s: &str| {
        let t =
            if project != module && project.len() >= 3 { s.replace(project.as_ref(), "{{name}}") } else { s.into() };
        swap_module(&t, &module)
    };
    let files = text_files(root)?
        .into_iter()
        .map(|f| TemplateFile { path: swap(&f.path), content: swap(&f.content) })
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Err(invalid("The project has no text files to save."));
    }
    let base = module_name(name);
    fs::create_dir_all(store)?;
    let mut id = base.clone();
    let mut n = 2;
    while store.join(&id).exists() {
        id = format!("{base}_{n}");
        n += 1;
    }
    let dir = store.join(&id);
    fs::create_dir(&dir)?;
    for f in &files {
        if !safe_template_path(&f.path) {
            return Err(invalid("A template filename isn't safe."));
        }
        let path = dir.join("files").join(&f.path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, &f.content)?;
    }
    crate::user_state::write_json(
        &dir.join("template.json"),
        &serde_json::json!({"name":name.trim(),"detail":detail}),
    )?;
    Ok(id)
}
pub fn list_user_templates(store: &Path) -> Vec<ProjectTemplate> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(store) else {
        return out;
    };
    for e in entries.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
            continue;
        }
        let dir = e.path();
        let Ok(meta) = fs::read_to_string(dir.join("template.json")) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&meta) else {
            continue;
        };
        let Some(name) = v["name"].as_str() else {
            continue;
        };
        if let Ok(files) = text_files(&dir.join("files")) {
            out.push(ProjectTemplate {
                id: format!("user:{}", e.file_name().to_string_lossy()),
                name: name.into(),
                detail: v["detail"].as_str().unwrap_or_default().into(),
                files,
                open: None,
                user: true,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
