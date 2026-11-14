//! Original project templates and their three supported substitutions.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateFile {
    pub path: String,
    pub content: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectTemplate {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub files: Vec<TemplateFile>,
    pub open: Option<String>,
    #[serde(default)]
    pub user: bool,
}
pub fn builtin_templates() -> Vec<ProjectTemplate> {
    serde_json::from_str(include_str!("templates_builtin.json")).expect("bundled templates are valid")
}
pub fn module_name(name: &str) -> String {
    let normalized =
        name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect::<String>();
    let mut words = normalized.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return "project".into();
    }
    if words[0].starts_with(|c: char| c.is_ascii_digit()) {
        let first = words.remove(0);
        words.push(first);
    }
    let id = words.join("_");
    if id.starts_with(|c: char| c.is_ascii_digit()) {
        format!("p_{id}")
    } else {
        id
    }
}
pub fn title_of(name: &str) -> String {
    name.replace(['-', '_'], " ")
        .split_whitespace()
        .map(|s| {
            let mut chars = s.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
    ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && ["COM", "LPT"].iter().any(|p| stem.starts_with(p))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
pub fn project_name_problem(name: &str) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        Some("Type a name for the project.")
    } else if n.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) || n.chars().any(char::is_control) {
        Some("A project name can't contain \\ / : * ? \" < > |")
    } else if n == "." || n == ".." || n.ends_with('.') || reserved(n) {
        Some("That name can't be used for a folder.")
    } else if n.chars().count() > 80 {
        Some("That name is too long.")
    } else {
        None
    }
}
pub fn safe_template_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && !path.starts_with(['\\', '/'])
        && !path.chars().any(char::is_control)
        && !path.contains([':', '*', '?', '"', '<', '>', '|'])
        && path
            .split(['\\', '/'])
            .all(|p| !p.is_empty() && p != "." && p != ".." && !p.ends_with(['.', ' ']) && !reserved(p))
}
pub fn fill(text: &str, name: &str) -> String {
    text.replace("{{name}}", name.trim())
        .replace("{{module}}", &module_name(name))
        .replace("{{title}}", &title_of(name))
}
pub fn render_template(t: &ProjectTemplate, name: &str) -> Vec<TemplateFile> {
    t.files.iter().map(|f| TemplateFile { path: fill(&f.path, name), content: fill(&f.content, name) }).collect()
}
