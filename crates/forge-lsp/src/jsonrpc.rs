//! JSON-RPC message framing used by the Language Server Protocol (and the
//! Debug Adapter Protocol): "Content-Length: <bytes>\r\n\r\n<json>".
//! Replaces `src/main/jsonrpc.ts`.

use serde_json::Value;

pub fn encode_message(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Collects bytes from a stream and returns each complete message.
#[derive(Default)]
pub struct MessageReader {
    buffer: Vec<u8>,
    body_length: Option<usize>,
    failed: bool,
}

const MAX_HEADER_BYTES: usize = 8192;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

impl MessageReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn feed(&mut self, mut chunk: &[u8]) -> Vec<Value> {
        let mut out = Vec::new();
        while !self.failed && !chunk.is_empty() {
            if let Some(length) = self.body_length {
                let take = (length - self.buffer.len()).min(chunk.len());
                self.buffer.extend_from_slice(&chunk[..take]);
                chunk = &chunk[take..];
                if self.buffer.len() == length {
                    if let Ok(value) = serde_json::from_slice(&self.buffer) {
                        out.push(value);
                    }
                    self.buffer.clear();
                    if self.buffer.capacity() > 64 * 1024 {
                        self.buffer = Vec::new();
                    }
                    self.body_length = None;
                }
                continue;
            }

            // Inspect each header byte once, even when the stream arrives one byte at a time.
            self.buffer.push(chunk[0]);
            chunk = &chunk[1..];
            if self.buffer.len() > MAX_HEADER_BYTES {
                self.failed = true;
            } else if self.buffer.ends_with(b"\r\n\r\n") {
                let length = std::str::from_utf8(&self.buffer).ok().and_then(|header| {
                    let mut lengths = header.lines().filter_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length").then_some(value.trim())
                    });
                    let length = lengths.next()?.parse::<usize>().ok()?;
                    (length > 0 && length <= MAX_BODY_BYTES && lengths.next().is_none()).then_some(length)
                });
                self.failed = length.is_none();
                self.body_length = length;
                self.buffer.clear();
            }
            if self.failed {
                // Framing is no longer trustworthy. The transport must close this stream.
                self.buffer = Vec::new();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hostile_lengths_and_unterminated_headers_fail_closed() {
        for header in [
            format!("Content-Length: {}\r\n\r\n", usize::MAX),
            "Content-Length: 67108865\r\n\r\n".into(),
            "x".repeat(8193),
        ] {
            let mut r = MessageReader::new();
            assert!(r.feed(header.as_bytes()).is_empty());
            // Once framing is lost, never reinterpret an attacker-controlled body as headers.
            assert!(r.feed(&encode_message(&json!({"id": 1}))).is_empty());
            assert!(r.buffer.is_empty(), "invalid stream retained memory");
        }
    }

    #[test]
    fn streams_large_bodies_and_many_messages_in_order() {
        let message = json!({"result": "x".repeat(128 * 1024)});
        let wire = encode_message(&message);
        let mut reader = MessageReader::new();
        let mut got = Vec::new();
        for chunk in wire.chunks(97) {
            got.extend(reader.feed(chunk));
        }
        assert_eq!(got, vec![message]);
        let wire: Vec<_> = (0..10_000).flat_map(|i| encode_message(&json!({"id": i}))).collect();
        let got = reader.feed(&wire);
        assert_eq!(got.len(), 10_000);
        assert_eq!(got[9999], json!({"id": 9999}));
    }

    #[test]
    fn content_length_counts_bytes_not_characters() {
        let msg = json!({ "jsonrpc": "2.0", "method": "x", "params": { "s": "é" } });
        let body = msg.to_string();
        let text = String::from_utf8(encode_message(&msg)).expect("utf8");
        assert_eq!(text, format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        assert_eq!(body.len(), body.chars().count() + 1);
    }

    #[test]
    fn reads_one_and_several_messages() {
        let mut r = MessageReader::new();
        assert_eq!(
            r.feed(&encode_message(&json!({ "id": 1, "result": "ok" }))),
            vec![json!({ "id": 1, "result": "ok" })]
        );
        let mut all = encode_message(&json!({ "a": 1 }));
        all.extend(encode_message(&json!({ "b": 2 })));
        all.extend(encode_message(&json!({ "c": 3 })));
        assert_eq!(r.feed(&all), vec![json!({ "a": 1 }), json!({ "b": 2 }), json!({ "c": 3 })]);
    }

    #[test]
    fn reassembles_a_message_split_at_any_byte() {
        let whole = encode_message(&json!({ "text": "naïve café ☕" }));
        for cut in 1..whole.len() {
            let mut r = MessageReader::new();
            let mut got = r.feed(&whole[..cut]);
            got.extend(r.feed(&whole[cut..]));
            assert_eq!(got, vec![json!({ "text": "naïve café ☕" })], "cut at {cut}");
        }
    }

    #[test]
    fn ignores_extra_headers() {
        let body = json!({ "ok": true }).to_string();
        let mut r = MessageReader::new();
        let framed = format!(
            "Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(r.feed(framed.as_bytes()), vec![json!({ "ok": true })]);
    }
}
