//! Filesystem boundary for Connect-generated files. Lexical validation and
//! canonical ancestors keep generation inside the chosen project. Bounded HTTP
//! test calls replace `src/main/apiCall.ts`; redirect checks keep keys on-site.
use std::{
    io,
    path::{Path, PathBuf},
};
pub fn project_path(root: &Path, relative: &str) -> io::Result<PathBuf> {
    if let Some(problem) = forge_core::names::name_problem(relative, true) {
        return Err(io::Error::other(problem));
    }
    let canonical_root = std::fs::canonicalize(root)?;
    let base = if root.is_absolute() { root.to_path_buf() } else { canonical_root.clone() };
    let path = relative.split(['/', '\\']).fold(base, |p, part| p.join(part));
    let mut existing = path.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| io::Error::other("Invalid project path"))?;
    }
    if !std::fs::canonicalize(existing)?.starts_with(&canonical_root) {
        return Err(io::Error::other("The target must stay inside the project."));
    }
    Ok(path)
}
#[derive(Clone, Debug, Default)]
pub struct TestCallResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub status_text: Option<String>,
    pub ms: f64,
    pub json: Option<serde_json::Value>,
    pub text: Option<String>,
    pub error: Option<String>,
    pub too_big: bool,
}
pub struct CallOptions {
    pub max_bytes: usize,
    pub timeout: std::time::Duration,
}
impl Default for CallOptions {
    fn default() -> Self {
        Self { max_bytes: 5 * 1024 * 1024, timeout: std::time::Duration::from_secs(20) }
    }
}
pub fn test_call(root: &Path, api: &forge_core::boxes::ApiBox, request_id: &str) -> TestCallResult {
    test_call_with_options(root, api, request_id, CallOptions::default())
}
pub fn test_call_with_options(
    root: &Path,
    api: &forge_core::boxes::ApiBox,
    request_id: &str,
    options: CallOptions,
) -> TestCallResult {
    use forge_core::{
        box_code::build_request,
        boxes::BoxAuth,
        dotenv::{merge_env, parse_dotenv},
    };
    use std::{collections::BTreeMap, io::Read, time::Instant};
    let started = Instant::now();
    let fail = |error: String| TestCallResult {
        error: Some(error),
        ms: started.elapsed().as_secs_f64() * 1000.0,
        ..Default::default()
    };
    let Some(req) = api.requests.iter().find(|r| r.id == request_id) else {
        return fail("That request is not in the box.".into());
    };
    let dotenv = std::fs::read_to_string(root.join(".env")).map(|s| parse_dotenv(&s)).unwrap_or_default();
    let env = merge_env(&std::env::vars().collect::<BTreeMap<_, _>>(), &dotenv, cfg!(windows));
    let built = match build_request(api, req, &env) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    // Validate before handing the URL to HTTP. Never include an auth-bearing
    // query string in transport/redirect errors returned to the UI or disk.
    let first = match url::Url::parse(&built.url) {
        Ok(u) if u.host_str().is_some() => u,
        _ => return fail("That URL isn't valid: check its host and path.".into()),
    };
    let mut url = first.clone();
    let mut method = built.method;
    let mut body = built.body;
    for hops in 0..=6 {
        if !matches!(url.scheme(), "http" | "https") {
            return fail("Only http:// and https:// addresses can be called.".into());
        }
        let Some(remaining) = options.timeout.checked_sub(started.elapsed()) else {
            return fail("No answer within 20 seconds.".into());
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(remaining))
            .build()
            .into();
        let mut builder = ureq::http::Request::builder().method(method.as_str()).uri(url.as_str());
        for (name, value) in &built.headers {
            builder = builder.header(name, value);
        }
        let request = match builder.body(body.as_deref().unwrap_or("")) {
            Ok(r) => r,
            Err(_) => return fail("The request method or headers aren't valid.".into()),
        };
        let mut response = match agent.run(request) {
            Ok(r) => r,
            Err(e) => {
                return fail(
                    if matches!(e, ureq::Error::Timeout(_)) {
                        "No answer within 20 seconds."
                    } else {
                        "The API request failed. Check the address and connection."
                    }
                    .into(),
                )
            }
        };
        let status = response.status();
        let mut result = TestCallResult {
            ok: status.is_success(),
            status: Some(status.as_u16()),
            status_text: Some(status.canonical_reason().unwrap_or("").into()),
            ..Default::default()
        };
        if status.is_redirection() {
            if let Some(location) = response.headers().get("location").and_then(|v| v.to_str().ok()) {
                if hops >= 5 {
                    return fail("Too many redirects.".into());
                }
                let next = match url.join(location) {
                    Ok(u) => u,
                    Err(_) => return fail("The API returned an invalid redirect address.".into()),
                };
                if next.origin() != first.origin() && !matches!(api.auth, BoxAuth::None) {
                    result.error=Some(format!("The API redirected to {}, another site, so Forge didn't send your key there. If that's right, use {} as the URL.",next.host_str().unwrap_or("another site"),next.origin().ascii_serialization()));
                    result.ms = started.elapsed().as_secs_f64() * 1000.0;
                    return result;
                }
                if status.as_u16() == 303 || (matches!(status.as_u16(), 301 | 302) && method == "POST") {
                    method = "GET".into();
                    body = None;
                }
                url = next;
                continue;
            }
        }
        let mut bytes = Vec::new();
        if response
            .body_mut()
            .as_reader()
            .take(options.max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return fail("Couldn't read the API response within the time limit.".into());
        }
        result.ms = started.elapsed().as_secs_f64() * 1000.0;
        if bytes.len() > options.max_bytes {
            result.ok = false;
            result.too_big = true;
            result.error = Some(format!("The response is over {} bytes: too big for a test call.", options.max_bytes));
            return result;
        }
        match serde_json::from_slice(&bytes) {
            Ok(json) => result.json = Some(json),
            Err(_) => result.text = Some(String::from_utf8_lossy(&bytes).chars().take(2000).collect()),
        }
        return result;
    }
    fail("Too many redirects.".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::boxes::{new_api_box, BoxAuth};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };
    fn server() -> (String, Arc<Mutex<String>>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let copy = seen.clone();
        let worker = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                let mut bytes = Vec::new();
                let mut b = [0; 1];
                while stream.read(&mut b).unwrap_or(0) > 0 {
                    bytes.push(b[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("");
                *copy.lock().unwrap() = request.clone();
                if path == "/stop" {
                    break;
                }
                let (status, header, body) = match path {
                    "/users" => ("200 OK", String::new(), "[{\"login\":\"ann\"},{\"login\":\"bo\"}]".to_owned()),
                    "/hop" => ("302 Found", "Location: /users\r\n".into(), String::new()),
                    "/away" => {
                        ("302 Found", format!("Location: http://localhost:{}/users\r\n", addr.port()), String::new())
                    }
                    "/loop" => ("302 Found", "Location: /loop\r\n".into(), String::new()),
                    "/text" => ("200 OK", String::new(), "hello there".to_owned()),
                    "/big" => ("200 OK", String::new(), "x".repeat(5000)),
                    _ => ("404 Not Found", String::new(), "{\"message\":\"Not Found\"}".into()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (format!("http://{addr}"), seen, worker)
    }
    #[test]
    fn test_calls_json_text_errors_redirects_and_limits() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".env"), "FORGE_CONNECT_TEST_TOKEN=from-dotenv\n").unwrap();
        let (base, seen, worker) = server();
        let mut api = new_api_box(&[], "Test");
        api.base_url = base.clone();
        api.auth = BoxAuth::Bearer { env: "FORGE_CONNECT_TEST_TOKEN".into() };
        api.requests[0].url = "/users".into();
        let call = |api: &forge_core::boxes::ApiBox| {
            test_call_with_options(root.path(), api, &api.requests[0].id, CallOptions::default())
        };
        let r = call(&api);
        assert!(r.ok, "{r:?}");
        assert_eq!(r.status, Some(200));
        assert_eq!(r.json, Some(serde_json::json!([{"login":"ann"},{"login":"bo"}])));
        assert!(seen.lock().unwrap().to_lowercase().contains("authorization: bearer from-dotenv"));
        api.requests[0].url = "/nope".into();
        let r = call(&api);
        assert!(!r.ok);
        assert_eq!(r.status, Some(404));
        assert!(r.json.is_some());
        api.requests[0].url = "/text".into();
        assert_eq!(call(&api).text.as_deref(), Some("hello there"));
        api.requests[0].url = "/hop".into();
        assert!(call(&api).ok);
        api.requests[0].url = "/away".into();
        assert!(call(&api).error.unwrap().contains("redirected to localhost"));
        assert!(seen.lock().unwrap().starts_with("GET /away "));
        api.requests[0].url = "/loop".into();
        assert_eq!(call(&api).error.as_deref(), Some("Too many redirects."));
        api.requests[0].url = "/big".into();
        let r = test_call_with_options(
            root.path(),
            &api,
            &api.requests[0].id,
            CallOptions { max_bytes: 1000, ..Default::default() },
        );
        assert!(r.too_big);
        assert!(r.json.is_none());
        api.base_url.clear();
        api.requests[0].url = "https://exa mple.com/x".into();
        assert!(call(&api).error.unwrap().contains("isn't valid"));
        api.requests[0].url = "file:///etc/passwd".into();
        assert!(call(&api).error.unwrap().contains("http"));
        api.base_url = base.clone();
        api.requests[0].url = "/users".into();
        api.auth = BoxAuth::Bearer { env: "FORGE_CONNECT_UNSET".into() };
        assert_eq!(call(&api).error.as_deref(), Some("FORGE_CONNECT_UNSET is not set: add it to .env"));
        assert!(test_call_with_options(root.path(), &api, "missing", CallOptions::default())
            .error
            .unwrap()
            .contains("request"));
        let mut stream = std::net::TcpStream::connect(base.trim_start_matches("http://")).unwrap();
        stream.write_all(b"GET /stop HTTP/1.1\r\n\r\n").unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn generated_paths_stay_in_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = project_path(dir.path(), "api/client.py").unwrap();
        assert!(path.ends_with("api/client.py"));
        for bad in ["../outside.py", "/outside.py", "C:\\outside.py", "api/../../outside.py"] {
            assert!(project_path(dir.path(), bad).is_err());
        }
    }
}
