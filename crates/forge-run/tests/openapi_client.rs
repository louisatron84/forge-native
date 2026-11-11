//! The original OpenAPI client execution case, with real Python and local HTTP.
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
#[test]
fn generated_openapi_client_calls_endpoints_with_auth_and_returns_nested_models() {
    let root = tempfile::tempdir().unwrap();
    let Some(python) = forge_run::python::find_python(root.path(), None) else { return };
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../forge-core/tests/fixtures/openapi.json")).unwrap();
    let spec = forge_core::openapi::read_spec(&cases[0]["doc"], "").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        for _ in 0..4 {
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "Client never sent all requests");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut data = Vec::new();
            let mut b = [0; 1];
            while stream.read(&mut b).unwrap_or(0) > 0 {
                data.push(b[0]);
                if data.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let headers = String::from_utf8(data).unwrap();
            let len = headers
                .lines()
                .find_map(|l| {
                    l.to_lowercase().strip_prefix("content-length:").and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut body = vec![0; len];
            stream.read_exact(&mut body).unwrap();
            assert!(headers.to_lowercase().contains("authorization: bearer tok"));
            assert!(headers.to_lowercase().contains("x-api-key: k1"));
            let first = headers.lines().next().unwrap().to_owned();
            let response = if first.starts_with("DELETE") {
                None
            } else if first.starts_with("GET /v1/pets/") {
                assert!(headers.to_lowercase().contains("x-trace: t1"));
                Some(r#"{"id":7,"name":"Rex","owner":{"name":"Ann"},"photo-urls":["a"],"class":"dog"}"#)
            } else if first.starts_with("POST") {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                    serde_json::json!({"name":"Ann","petName":"Tom"})
                );
                Some(r#"{"id":8,"name":"Tom"}"#)
            } else {
                Some(r#"[{"id":1,"name":"A"},{"id":2,"name":"B","tag":"x"}]"#)
            };
            let (status, body) = response.map(|s| ("200 OK", s)).unwrap_or(("204 No Content", ""));
            write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                .unwrap();
            seen.push(first);
        }
        seen
    });
    std::fs::create_dir(root.path().join("api")).unwrap();
    std::fs::write(root.path().join("api/petstore_client.py"), forge_core::openapi_code::generate_client(&spec, None))
        .unwrap();
    let script="from api.petstore_client import list_pets, create_pet, show_pet_by_id, delete_pets_by, NewPet\npets = list_pets(limit=2)\nprint([p.name for p in pets], pets[1].tag)\nmade = create_pet(NewPet(pet_name='Tom', name='Ann'))\nprint(made.id, made.name)\nrex = show_pet_by_id('a b', x_trace='t1')\nprint(rex.owner.name, rex.photo_urls, rex.class_)\nprint(delete_pets_by('7'))\n";
    let base = format!("http://{addr}/v1");
    let result = forge_sys::process::run_captured(
        &python,
        &["-c".into(), script.into()],
        root.path(),
        &[("PETSTORE_BASE_URL", &base), ("PETSTORE_TOKEN", "tok"), ("PETSTORE_API_KEY", "k1")],
        "",
        Duration::from_secs(10),
    );
    assert!(result.ok(), "{result:?}");
    assert_eq!(result.stdout, "['A', 'B'] x\n8 Tom\nAnn ['a'] dog\nNone\n");
    assert_eq!(
        worker.join().unwrap(),
        [
            "GET /v1/pets?limit=2 HTTP/1.1",
            "POST /v1/pets HTTP/1.1",
            "GET /v1/pets/a%20b HTTP/1.1",
            "DELETE /v1/pets/7 HTTP/1.1"
        ]
    );
}
