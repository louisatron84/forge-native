//! Checks for file and folder names typed by the user. Replaces
//! `src/shared/names.ts`.

fn windows_reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or("").to_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// Why a name typed in the tree can't be used, or `None` if it's fine.
/// `allow_nested` lets "pkg/mod.py" create the folder "pkg" on the way.
pub fn name_problem(name: &str, allow_nested: bool) -> Option<String> {
    if name.trim().is_empty() {
        return Some("A name is required.".into());
    }
    let parts: Vec<&str> = name.split(['/', '\\']).collect();
    if parts.len() > 1 && !allow_nested {
        return Some("The name can't contain / or \\.".into());
    }
    let b = name.as_bytes();
    if name.starts_with(['/', '\\']) || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
        return Some("Use a name inside this folder, not a full path.".into());
    }
    for part in parts {
        if part.is_empty() {
            return Some("The name has an empty part (two slashes in a row, or a slash at the end).".into());
        }
        if part == "." || part == ".." {
            return Some(format!("\"{part}\" isn't allowed as a name."));
        }
        if part.chars().any(|c| "<>:\"|?*".contains(c) || (c as u32) < 0x20) {
            return Some("Names can't contain < > : \" | ? *".into());
        }
        if part.ends_with(['.', ' ']) {
            return Some("Names can't end with a dot or a space.".into());
        }
        if windows_reserved(part) {
            return Some(format!("\"{part}\" is a reserved name on Windows."));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(name: &str) -> String {
        name_problem(name, true).unwrap_or_default()
    }

    #[test]
    fn accepts_normal_and_nested_names() {
        for ok in ["main.py", ".env", "pkg/mod.py", "pkg\\mod.py", "console.py", "com10.txt"] {
            assert_eq!(name_problem(ok, true), None, "{ok}");
        }
    }

    #[test]
    fn rejects_empty_odd_and_unsafe_names() {
        assert!(problem("").contains("required"));
        assert!(problem("   ").contains("required"));
        assert!(problem("ab:c").contains("can't contain"));
        assert!(problem("what?.py").contains("can't contain"));
        assert!(problem("..").contains("isn't allowed"));
        assert!(problem("../outside.py").contains("isn't allowed"));
        assert!(problem("a//b").contains("empty part"));
        assert!(problem("folder/").contains("empty part"));
        assert!(problem("/abs.py").contains("full path"));
        assert!(problem("C:\\x.py").contains("full path"));
        assert!(problem("name.").contains("end with"));
        assert!(problem("CON").contains("reserved"));
        assert!(problem("nul.txt").contains("reserved"));
        assert!(problem("com1").contains("reserved"));
    }

    #[test]
    fn can_forbid_slashes_for_rename() {
        assert!(name_problem("a/b", false).unwrap_or_default().contains("can't contain /"));
    }
}
