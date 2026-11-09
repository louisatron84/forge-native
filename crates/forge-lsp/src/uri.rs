//! File path <-> file:// URI, as the Language Server Protocol uses them.
//! Replaces `src/shared/uri.ts` (same encoding: the drive colon is encoded,
//! like VS Code does).

fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn path_to_uri(path: &str) -> String {
    let b = path.as_bytes();
    let win = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    let mut p = path.replace('\\', "/");
    if win {
        p = format!("/{}:{}", p[..1].to_lowercase(), &p[2..]);
    }
    let encoded: Vec<String> = p.split('/').map(encode_component).collect();
    format!("file://{}", encoded.join("/"))
}

pub fn uri_to_path(uri: &str) -> String {
    let p = decode(uri.strip_prefix("file://").unwrap_or(uri));
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return format!("{}:{}", p[1..2].to_uppercase(), p[3..].replace('/', "\\"));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_windows_paths_encoding_the_drive_colon_and_spaces() {
        let uri = path_to_uri("C:\\Users\\louis\\My Project\\app.py");
        assert_eq!(uri, "file:///c%3A/Users/louis/My%20Project/app.py");
        assert_eq!(uri_to_path(&uri), "C:\\Users\\louis\\My Project\\app.py");
    }

    #[test]
    fn accepts_the_unencoded_drive_form() {
        assert_eq!(uri_to_path("file:///C:/a/b.py"), "C:\\a\\b.py");
    }

    #[test]
    fn converts_posix_paths() {
        assert_eq!(path_to_uri("/home/me/a b.py"), "file:///home/me/a%20b.py");
        assert_eq!(uri_to_path("file:///home/me/a%20b.py"), "/home/me/a b.py");
    }

    #[test]
    fn encodes_url_special_characters_and_non_ascii() {
        let p = "C:\\x\\#hash%?.py";
        assert_eq!(uri_to_path(&path_to_uri(p)), p);
        let q = "/home/me/café ☕.py";
        assert_eq!(uri_to_path(&path_to_uri(q)), q);
    }
}
