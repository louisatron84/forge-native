//! The project's Python dependencies: what requirements.txt / pyproject.toml
//! ask for, compared with what's installed in the interpreter, so missing
//! packages can be pointed out and installed. Replaces `src/shared/deps.ts`.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// As written (e.g. "Django").
    pub name: String,
    /// The rest of the line: version constraints, extras, markers ("" if none).
    pub spec: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
}

/// Package names compare case-insensitively with -, _ and . the same (PEP 503).
pub fn normalize_name(name: &str) -> String {
    static SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[-_.]+").expect("regex"));
    SEP.replace_all(&name.trim().to_lowercase(), "-").into_owned()
}

static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?)").expect("regex"));
static EGG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[#&]egg=([A-Za-z0-9._-]+)").expect("regex"));
static URL_OR_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\w+\+)?(https?|file|git|ssh):|^[.~/\\]|^[A-Za-z]:[\\/]").expect("regex"));

/// One requirement string ("requests[socks]>=2; python_version>'3.8'") as name + spec.
pub fn parse_requirement(text: &str) -> Option<Requirement> {
    let t = text.trim();
    // URLs and paths: only usable if they name the package with #egg=
    if URL_OR_PATH.is_match(t) {
        return EGG.captures(t).map(|c| Requirement { name: c[1].to_owned(), spec: String::new() });
    }
    let m = NAME.find(t)?;
    // "name @ url" is a direct reference: still that package.
    Some(Requirement { name: m.as_str().to_owned(), spec: t[m.end()..].trim().to_owned() })
}

/// requirements.txt as requirements (skips comments, options like -r / --index-url, and URLs without #egg=).
pub fn parse_requirements_txt(text: &str) -> Vec<Requirement> {
    static CONT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\\r?\n").expect("regex"));
    static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\s)#.*$").expect("regex"));
    static EDITABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(-e|--editable)\s+(.+)$").expect("regex"));
    let joined = CONT.replace_all(text, " ");
    let mut out: Vec<Requirement> = Vec::new();
    for raw in joined.lines() {
        let line = COMMENT.replace(raw, "").trim().to_owned();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('-') {
            // -e git+...#egg=name is an editable install of a named package.
            if let Some(req) = EDITABLE.captures(&line).and_then(|c| parse_requirement(&c[2])) {
                out.push(req);
            }
            continue;
        }
        if let Some(req) = parse_requirement(&line) {
            if !out.iter().any(|r| normalize_name(&r.name) == normalize_name(&req.name)) {
                out.push(req);
            }
        }
    }
    out
}

/// The `[project]` dependencies list from pyproject.toml (not a full TOML
/// parser: it reads the `dependencies = [ "...", ... ]` array, which is what
/// packaging tools write).
pub fn parse_pyproject_dependencies(text: &str) -> Vec<Requirement> {
    static HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[([^\]]+)\]\s*(#.*)?$").expect("regex"));
    static STRINGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""((?:[^"\\]|\\.)*)"|'([^']*)'"#).expect("regex"));
    static CLOSED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\s*(#.*)?$").expect("regex"));
    let mut out = Vec::new();
    let mut table = String::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_start();
        if let Some(c) = HEADER.captures(line) {
            table = c[1].trim().to_owned();
            i += 1;
            continue;
        }
        let is_deps = line.strip_prefix("dependencies").is_some_and(|r| r.trim_start().starts_with('='));
        if table == "project" && is_deps {
            // Collect the array, which may span several lines.
            let mut body = line[line.find('=').map_or(0, |p| p + 1)..].to_owned();
            while !CLOSED.is_match(&STRINGS.replace_all(&body, "\"\"")) && i + 1 < lines.len() {
                i += 1;
                body.push('\n');
                body.push_str(lines[i]);
            }
            for c in STRINGS.captures_iter(&body) {
                let s = c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str());
                if let Some(req) = parse_requirement(s) {
                    out.push(req);
                }
            }
        }
        i += 1;
    }
    out
}

/// Requirements that aren't installed (by normalized name; versions aren't checked).
pub fn missing_packages(required: &[Requirement], installed: &[InstalledPackage]) -> Vec<Requirement> {
    let have: Vec<String> = installed.iter().map(|p| normalize_name(&p.name)).collect();
    required.iter().filter(|r| !have.contains(&normalize_name(&r.name))).cloned().collect()
}

/// Parse `pip list --format=json` output.
pub fn parse_pip_list(json: &str) -> Vec<InstalledPackage> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    items.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()
}

/// A name typed for "Install a package…": letters, digits, ._- and optional [extras] / version spec.
pub fn is_installable_spec(text: &str) -> bool {
    static SPEC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*(\[[A-Za-z0-9._,-]+\])?\s*([<>=!~]=?\s*[\w.*+!-]+\s*,?\s*)*$")
            .expect("regex")
    });
    SPEC.is_match(text.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str, spec: &str) -> Requirement {
        Requirement { name: name.into(), spec: spec.into() }
    }

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize_name("Django_REST.framework"), "django-rest-framework");
    }

    #[test]
    fn splits_the_name_from_the_spec() {
        assert_eq!(
            parse_requirement("requests[socks]>=2.0; python_version > \"3.8\""),
            Some(req("requests", "[socks]>=2.0; python_version > \"3.8\""))
        );
        assert_eq!(parse_requirement("numpy"), Some(req("numpy", "")));
        assert_eq!(parse_requirement("pkg @ https://x/pkg.whl"), Some(req("pkg", "@ https://x/pkg.whl")));
        assert_eq!(parse_requirement("git+https://github.com/a/b.git#egg=bee"), Some(req("bee", "")));
        assert_eq!(parse_requirement("https://x/y.whl"), None);
        assert_eq!(parse_requirement("./local/pkg"), None);
    }

    #[test]
    fn reads_requirements_txt() {
        let text = [
            "# web",
            "Flask>=2.0  # the framework",
            "requests==2.31.0",
            "-r other.txt",
            "--index-url https://example.org/simple",
            "-e git+https://github.com/a/b.git#egg=bee",
            "numpy \\",
            "  >=1.26",
            "",
            "flask",
        ]
        .join("\n");
        assert_eq!(
            parse_requirements_txt(&text),
            vec![req("Flask", ">=2.0"), req("requests", "==2.31.0"), req("bee", ""), req("numpy", ">=1.26")]
        );
    }

    #[test]
    fn reads_pyproject_dependencies() {
        let text = "\n[build-system]\nrequires = [\"setuptools\"]\ndependencies = [\"not-this\"]\n\n[project]\nname = \"demo\"\ndependencies = [\n  \"fastapi>=0.110\",  # web\n  'uvicorn[standard]',\n  \"pydantic; python_version >= '3.9'\",\n]\n\n[project.optional-dependencies]\ndev = [\"pytest\"]\n";
        let names: Vec<String> = parse_pyproject_dependencies(text).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["fastapi", "uvicorn", "pydantic"]);
        let one: Vec<String> = parse_pyproject_dependencies("[project]\ndependencies = [\"a\", \"b>=1\"]\n")
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(one, ["a", "b"]);
    }

    #[test]
    fn compares_by_normalized_name() {
        let installed = vec![
            InstalledPackage { name: "flask".into(), version: "3.0".into() },
            InstalledPackage { name: "python-dateutil".into(), version: "2.9".into() },
        ];
        let missing = missing_packages(&[req("Flask", ""), req("python_dateutil", ""), req("numpy", "")], &installed);
        assert_eq!(missing, vec![req("numpy", "")]);
    }

    #[test]
    fn reads_pip_list_and_tolerates_garbage() {
        assert_eq!(
            parse_pip_list(r#"[{"name":"pip","version":"24.0"},{"x":1}]"#),
            vec![InstalledPackage { name: "pip".into(), version: "24.0".into() }]
        );
        assert!(parse_pip_list("WARNING: blah").is_empty());
    }

    #[test]
    fn accepts_package_names_with_extras_and_versions_only() {
        assert!(is_installable_spec("requests"));
        assert!(is_installable_spec("uvicorn[standard]>=0.29"));
        assert!(is_installable_spec("numpy==1.26.4"));
        assert!(!is_installable_spec("rm -rf /"));
        assert!(!is_installable_spec("a; echo hi"));
        assert!(!is_installable_spec(""));
    }
}
