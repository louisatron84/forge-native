//! Real standard-library execution coverage for Connect's generated Python.
use forge_core::{
    box_code::generate_block,
    boxes::{new_api_box, BoxAuth},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
#[test]
fn generated_api_client_calls_local_server_and_writes_selected_csv_fields() {
    let root = tempfile::tempdir().unwrap();
    let Some(python) = forge_run::python::find_python(root.path(), None) else { return };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "Generated client never called the local API");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut request = Vec::new();
        let mut byte = [0; 1];
        while stream.read(&mut byte).unwrap_or(0) > 0 {
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8(request).unwrap();
        assert!(text.starts_with("GET /users/forge "));
        assert!(text.to_lowercase().contains("authorization: bearer local-test-token"));
        let body = r#"{"users":[{"login":"ann","plan":{"name":"pro"},"hidden":1},{"login":"bo","plan":{"name":"free"},"hidden":2}]}"#;
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    });
    let mut api = new_api_box(&[], "Local");
    api.base_url = format!("http://{address}");
    api.auth = BoxAuth::Bearer { env: "FORGE_LOCAL_CLIENT_TOKEN".into() };
    let req = &mut api.requests[0];
    req.url = "/users/{org}".into();
    req.test_args.insert("org".into(), "forge".into());
    req.records_path = Some(vec!["users".into()]);
    req.ticked = vec!["login".into(), "plan.name".into()];
    req.output = "users.csv".into();
    std::fs::write(root.path().join("client.py"), generate_block(&api, true)).unwrap();
    let result = forge_sys::process::run_captured(
        &python,
        &["client.py".into()],
        root.path(),
        &[("FORGE_LOCAL_CLIENT_TOKEN", "local-test-token")],
        "",
        Duration::from_secs(10),
    );
    assert!(result.ok(), "{result:?}");
    worker.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("users.csv")).unwrap(),
        "login,plan.name\r\nann,pro\r\nbo,free\r\n"
    );
}
