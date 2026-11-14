//! Generated clients must keep authorization headers on the original origin.
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};

#[test]
fn generated_clients_follow_same_origin_but_block_cross_origin_redirects() {
    let root = tempfile::tempdir().unwrap();
    let Some(python) = forge_run::python::find_python(root.path(), None) else { return };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let mut leaked = false;
        for connection in listener.incoming() {
            let mut stream = connection.unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0];
            while stream.read(&mut byte).unwrap_or(0) > 0 {
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&bytes);
            let path = request.split_whitespace().nth(1).unwrap_or("");
            if path == "/stop" {
                break;
            }
            let (status, location, body) = match path {
                "/same" => ("302 Found", "Location: /ok\r\n".into(), ""),
                "/away" => ("302 Found", format!("Location: http://localhost:{}/leak\r\n", address.port()), ""),
                "/leak" => {
                    leaked = true;
                    ("200 OK", String::new(), "{}")
                }
                _ => ("200 OK", String::new(), "{\"ok\":true}"),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        leaked
    });
    let base = format!("http://{address}");
    let mut api = forge_core::boxes::new_api_box(&[], "Local");
    api.base_url = base.clone();
    std::fs::write(root.path().join("box.py"), forge_core::box_code::generate_block(&api, false)).unwrap();
    let spec = forge_core::openapi::read_spec(
        &serde_json::json!({
            "openapi":"3.0.0","info":{"title":"Local","version":"1"},
            "servers":[{"url":base}],"paths":{}
        }),
        "",
    )
    .unwrap();
    std::fs::write(root.path().join("client.py"), forge_core::openapi_code::generate_client(&spec, None)).unwrap();
    let script = "import box, client\nfor call in (box._local_request, client._request):\n    assert call('GET', '/same', headers={'X-Private-Token':'secret'}) == {'ok': True}\n    try:\n        call('GET', '/away', headers={'X-Private-Token':'secret'})\n    except RuntimeError as error:\n        assert 'redirect' in str(error).lower()\n    else:\n        raise AssertionError('cross-origin redirect was followed')\n";
    let result = forge_sys::process::run_captured(
        &python,
        &["-c".into(), script.into()],
        root.path(),
        &[],
        "",
        Duration::from_secs(10),
    );
    let mut stop = std::net::TcpStream::connect(address).unwrap();
    write!(stop, "GET /stop HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let leaked = worker.join().unwrap();
    assert!(!leaked, "generated client sent credentials to another origin");
    assert!(result.ok(), "{result:?}");
}
