//! User keyboard shortcuts (`<data>/keybindings.json`). Replaces
//! `src/shared/keybindings.ts`.
//!
//! The file maps command ids to keys and only lists the commands you changed:
//!   { "run.file": "Ctrl+R", "palette.files": ["Ctrl+P", "Ctrl+E"], "nav.back": null }
//! A string or list replaces the command's default keys; null (or []) removes them.
//! Keys are written as the old app wrote them ("Ctrl+Shift+P", "Alt+ArrowLeft").

use serde_json::Value;
use std::collections::BTreeMap;

pub type KeyOverrides = BTreeMap<String, Vec<String>>;

fn modifier(p: &str) -> Option<&'static str> {
    match p {
        "ctrl" | "control" | "cmd" | "command" | "meta" | "cmdorctrl" | "commandorcontrol" => Some("Ctrl"),
        "shift" => Some("Shift"),
        "alt" | "option" => Some("Alt"),
        _ => None,
    }
}

fn key_name(p: &str) -> Option<&'static str> {
    Some(match p {
        "esc" | "escape" => "Escape",
        "enter" | "return" => "Enter",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "up" | "arrowup" => "ArrowUp",
        "down" | "arrowdown" => "ArrowDown",
        "left" | "arrowleft" => "ArrowLeft",
        "right" | "arrowright" => "ArrowRight",
        "plus" => "+",
        "backquote" => "`",
        _ => return None,
    })
}

fn is_function_key(p: &str) -> bool {
    let lower = p.to_lowercase();
    let Some(n) = lower.strip_prefix('f') else { return false };
    n.parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) && !n.starts_with('0')
}

/// Write a shortcut canonically ('ctrl+shift+p' -> 'Ctrl+Shift+P'), or `None`
/// if it isn't a usable shortcut.
pub fn normalize_key(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }
    // Split on '+', but a trailing '+' is the key itself (Ctrl++).
    let parts: Vec<String> = if let Some(rest) = text.strip_suffix("++") {
        rest.split('+').map(str::to_owned).chain(std::iter::once("+".to_owned())).collect()
    } else {
        text.split('+').map(str::to_owned).collect()
    };
    let mut mods = std::collections::BTreeSet::new();
    let mut key: Option<String> = None;
    for raw in &parts {
        let p = raw.trim();
        if p.is_empty() {
            return None;
        }
        if let Some(m) = modifier(&p.to_lowercase()) {
            mods.insert(m);
            continue;
        }
        if key.is_some() {
            return None; // two non-modifier keys
        }
        key = Some(if let Some(named) = key_name(&p.to_lowercase()) {
            named.to_owned()
        } else if is_function_key(p) || p.chars().count() == 1 {
            p.to_uppercase()
        } else {
            return None;
        });
    }
    let key = key?;
    let mut out: Vec<String> =
        ["Ctrl", "Shift", "Alt"].iter().filter(|m| mods.contains(*m)).map(|m| (*m).to_owned()).collect();
    out.push(key);
    Some(out.join("+"))
}

fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Read keybindings.json content (anything; bad entries are dropped) into overrides.
pub fn normalize_keybindings(raw: &Value) -> KeyOverrides {
    let mut out = KeyOverrides::new();
    let Some(obj) = raw.as_object() else { return out };
    for (id, value) in obj {
        if !plain_id(id) {
            continue;
        }
        if value.is_null() {
            out.insert(id.clone(), Vec::new());
            continue;
        }
        let list: Vec<&Value> = match value {
            Value::String(_) => vec![value],
            Value::Array(items) => items.iter().collect(),
            _ => continue,
        };
        let mut keys: Vec<String> = Vec::new();
        for v in &list {
            if let Some(k) = v.as_str().and_then(normalize_key) {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        // A list of only bad keys is a typo, not "remove the shortcut".
        if !keys.is_empty() || list.is_empty() {
            out.insert(id.clone(), keys);
        }
    }
    out
}

/// The JSON written back: [] as null, one key as a string, several as a list.
pub fn keybindings_json(overrides: &KeyOverrides) -> Value {
    let mut map = serde_json::Map::new();
    for (id, keys) in overrides {
        let v = match keys.len() {
            0 => Value::Null,
            1 => Value::String(keys[0].clone()),
            _ => Value::Array(keys.iter().cloned().map(Value::String).collect()),
        };
        map.insert(id.clone(), v);
    }
    Value::Object(map)
}

/// Keys used by more than one command: key -> command ids (in order).
pub fn find_conflicts(bindings: &[(String, Vec<String>)]) -> Vec<(String, Vec<String>)> {
    let mut by_key: Vec<(String, Vec<String>)> = Vec::new();
    for (id, keys) in bindings {
        for k in keys {
            match by_key.iter_mut().find(|(key, _)| key == k) {
                Some((_, ids)) => {
                    if !ids.contains(id) {
                        ids.push(id.clone());
                    }
                }
                None => by_key.push((k.clone(), vec![id.clone()])),
            }
        }
    }
    by_key.retain(|(_, ids)| ids.len() >= 2);
    by_key
}

/// A shortcut in the form GPUI key bindings use: 'Ctrl+Shift+P' -> 'ctrl-shift-p'.
pub fn to_gpui_keystroke(key: &str) -> String {
    let parts: Vec<&str> = if let Some(rest) = key.strip_suffix("++") {
        rest.split('+').chain(std::iter::once("+")).collect()
    } else {
        key.split('+').collect()
    };
    parts
        .iter()
        .map(|p| match *p {
            "Ctrl" => "ctrl".to_owned(),
            "Shift" => "shift".to_owned(),
            "Alt" => "alt".to_owned(),
            "ArrowUp" => "up".to_owned(),
            "ArrowDown" => "down".to_owned(),
            "ArrowLeft" => "left".to_owned(),
            "ArrowRight" => "right".to_owned(),
            "PageUp" => "pageup".to_owned(),
            "PageDown" => "pagedown".to_owned(),
            "+" => "+".to_owned(),
            other => other.to_lowercase(),
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_shortcuts_the_way_key_events_produce_them() {
        assert_eq!(normalize_key("ctrl+shift+p").as_deref(), Some("Ctrl+Shift+P"));
        assert_eq!(normalize_key("Shift+Ctrl+P").as_deref(), Some("Ctrl+Shift+P"));
        assert_eq!(normalize_key("cmd+k").as_deref(), Some("Ctrl+K"));
        assert_eq!(normalize_key(" alt + left ").as_deref(), Some("Alt+ArrowLeft"));
        assert_eq!(normalize_key("f5").as_deref(), Some("F5"));
        assert_eq!(normalize_key("Ctrl+`").as_deref(), Some("Ctrl+`"));
        assert_eq!(normalize_key("Ctrl++").as_deref(), Some("Ctrl++"));
        assert_eq!(normalize_key("ctrl+pagedown").as_deref(), Some("Ctrl+PageDown"));
        assert_eq!(normalize_key("Escape").as_deref(), Some("Escape"));
        assert_eq!(normalize_key("ctrl+space").as_deref(), Some("Ctrl+Space"));
    }

    #[test]
    fn rejects_things_that_are_not_one_key() {
        for bad in ["", "Ctrl", "Ctrl+A+B", "Ctrl+Banana", "F99"] {
            assert_eq!(normalize_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn accepts_a_key_a_list_of_keys_or_null_to_remove() {
        let out = normalize_keybindings(
            &json!({ "run.file": "ctrl+r", "palette.files": ["Ctrl+P", "ctrl+e", "Ctrl+P"], "nav.back": null }),
        );
        assert_eq!(out.get("run.file"), Some(&vec!["Ctrl+R".to_owned()]));
        assert_eq!(out.get("palette.files"), Some(&vec!["Ctrl+P".to_owned(), "Ctrl+E".to_owned()]));
        assert_eq!(out.get("nav.back"), Some(&vec![]));
    }

    #[test]
    fn drops_garbage_instead_of_failing() {
        assert!(normalize_keybindings(&json!("nope")).is_empty());
        assert!(normalize_keybindings(&json!([1, 2])).is_empty());
        assert!(normalize_keybindings(&json!({ "a.b": 42, "bad id!": "Ctrl+A", "c.d": ["Ctrl+Nope"] })).is_empty());
    }

    #[test]
    fn lists_keys_shared_by_several_commands() {
        let c = find_conflicts(&[
            ("a".into(), vec!["Ctrl+R".into(), "F1".into()]),
            ("b".into(), vec!["Ctrl+R".into()]),
            ("c".into(), vec!["F2".into()]),
        ]);
        assert_eq!(c, vec![("Ctrl+R".to_owned(), vec!["a".to_owned(), "b".to_owned()])]);
    }

    #[test]
    fn converts_to_gpui_keystrokes() {
        assert_eq!(to_gpui_keystroke("Ctrl+Shift+P"), "ctrl-shift-p");
        assert_eq!(to_gpui_keystroke("Alt+ArrowLeft"), "alt-left");
        assert_eq!(to_gpui_keystroke("F5"), "f5");
        assert_eq!(to_gpui_keystroke("Ctrl+`"), "ctrl-`");
        assert_eq!(to_gpui_keystroke("Ctrl++"), "ctrl-+");
    }

    #[test]
    fn writes_overrides_back_as_the_old_app_did() {
        let mut o = KeyOverrides::new();
        o.insert("a".into(), vec![]);
        o.insert("b".into(), vec!["Ctrl+B".into()]);
        o.insert("c".into(), vec!["F1".into(), "F2".into()]);
        assert_eq!(keybindings_json(&o), json!({ "a": null, "b": "Ctrl+B", "c": ["F1", "F2"] }));
    }
}
