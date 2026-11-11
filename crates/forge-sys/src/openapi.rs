//! Bounded JSON/YAML loading for OpenAPI, replacing `main/openapiLoad.ts`.
use serde_json::Value;
use std::{io::Read, time::Duration};
pub const MAX_SPEC_BYTES: usize = 10 * 1024 * 1024;
#[derive(Debug)]
pub struct LoadedSpec {
    pub doc: Value,
    pub from: String,
}
pub fn parse_spec_text(text: &str) -> Result<Value, String> {
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text)
        .or_else(|_| serde_saphyr::from_str(text).map_err(|e| e.to_string()))
        .map_err(|e| format!("It isn't valid JSON or YAML: {}", e.lines().next().unwrap_or("Invalid document")))
}
pub fn load_spec(source: &str) -> Result<LoadedSpec, String> {
    load_spec_with_limit(source, MAX_SPEC_BYTES)
}
pub fn load_spec_with_limit(source: &str, max: usize) -> Result<LoadedSpec, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("Give the address or file of an OpenAPI description.".into());
    }
    let too_big = || "That description is too large (over 10 MB).".to_owned();
    let mut bytes = Vec::new();
    let lower = source.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .max_redirects(5)
            .build()
            .into();
        let mut response = agent
            .get(source)
            .header("Accept", "application/json, application/yaml, text/yaml, */*")
            .call()
            .map_err(|_| "Couldn't fetch it: check the address and connection, or try again.".to_owned())?;
        if !response.status().is_success() {
            return Err(format!(
                "The server answered {} {}.",
                response.status().as_u16(),
                response.status().canonical_reason().unwrap_or("")
            ));
        }
        if response
            .headers()
            .get("content-length")
            .and_then(|s| s.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n > max as u64)
        {
            return Err(too_big());
        }
        response
            .body_mut()
            .as_reader()
            .take(max.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Couldn't read the description within the time limit.".to_owned())?;
    } else {
        if source.contains("://") {
            return Err("Only http(s) addresses and files can be loaded.".into());
        }
        let file = std::fs::File::open(source).map_err(|e| format!("Couldn't read the file: {e}"))?;
        if file.metadata().map_err(|e| e.to_string())?.len() > max as u64 {
            return Err(too_big());
        }
        file.take(max.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Couldn't read the file: {e}"))?;
    }
    if bytes.len() > max {
        return Err(too_big());
    }
    let doc = parse_spec_text(&String::from_utf8_lossy(&bytes))?;
    Ok(LoadedSpec { doc, from: source.into() })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_json_yaml_bom_and_files_and_reports_bad_input() {
        assert_eq!(parse_spec_text("{\"openapi\":\"3.0.0\"}").unwrap(), serde_json::json!({"openapi":"3.0.0"}));
        assert_eq!(
            parse_spec_text("openapi: 3.0.0\ninfo:\n  title: X\n").unwrap(),
            serde_json::json!({"openapi":"3.0.0","info":{"title":"X"}})
        );
        assert_eq!(parse_spec_text("\u{feff}{\"a\":1}").unwrap(), serde_json::json!({"a":1}));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("api.yaml");
        std::fs::write(&file, "swagger: \"2.0\"\ninfo: {title: Shop}\n").unwrap();
        assert_eq!(load_spec(&file.to_string_lossy()).unwrap().doc["info"]["title"], "Shop");
        std::fs::write(&file, "a: [unclosed\n").unwrap();
        assert!(load_spec(&file.to_string_lossy()).unwrap_err().contains("isn't valid JSON or YAML"));
        assert!(load_spec(&dir.path().join("missing").to_string_lossy()).unwrap_err().contains("Couldn't read"));
        assert!(load_spec("ftp://x/y").unwrap_err().contains("Only http"));
        assert!(load_spec(" ").unwrap_err().contains("Give the address"));
    }
    #[test]
    fn http_fetch_success_error_and_streaming_limit() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        for (status, body, expected) in [
            ("200 OK", "{\"openapi\":\"3.1.0\"}", "ok"),
            ("404 Not Found", "no", "404 Not Found"),
            ("200 OK", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "too large"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let source = format!("http://{}/spec", listener.local_addr().unwrap());
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0; 4096];
                let _ = stream.read(&mut buf);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            });
            let result = load_spec_with_limit(&source, 32);
            if expected == "ok" {
                assert_eq!(result.unwrap().doc["openapi"], "3.1.0")
            } else {
                assert!(result.unwrap_err().contains(expected))
            }
            worker.join().unwrap();
        }
    }
}
