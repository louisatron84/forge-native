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
}

impl MessageReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Value> {
        self.buffer.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(header_end) = self.buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&self.buffer[..header_end]).to_lowercase();
            let length = header
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok());
            let Some(length) = length else {
                // Malformed header: drop it and resync on the next one.
                self.buffer.drain(..header_end + 4);
                continue;
            };
            let start = header_end + 4;
            if self.buffer.len() < start + length {
                break; // wait for the rest of the body
            }
            if let Ok(v) = serde_json::from_slice::<Value>(&self.buffer[start..start + length]) {
                out.push(v);
            }
            self.buffer.drain(..start + length);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
