//! A JSON-RPC client for the Language Server Protocol. It doesn't know how
//! messages travel (`process.rs` pipes them to a server); it matches responses
//! to requests and dispatches the rest. Replaces `src/renderer/lsp/client.ts`.

use futures::channel::oneshot;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

pub type Send = Arc<dyn Fn(Value) + std::marker::Send + Sync>;
type NotificationHandler = Box<dyn Fn(Value) + std::marker::Send + Sync>;
type RequestHandler = Box<dyn Fn(Value) -> Value + std::marker::Send + Sync>;

/// A request from the server that the app answers later (with `respond`).
pub type DeferredHandler = Box<dyn Fn(Value, Value) + std::marker::Send + Sync>;

pub struct LspClient {
    send: Send,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>,
    notifications: Mutex<HashMap<String, Vec<NotificationHandler>>>,
    requests: Mutex<HashMap<String, RequestHandler>>,
    deferred: Mutex<HashMap<String, DeferredHandler>>,
}

impl LspClient {
    pub fn new(send: Send) -> Arc<Self> {
        Arc::new(Self {
            send,
            next_id: AtomicI64::new(1),
            pending: Mutex::new(HashMap::new()),
            notifications: Mutex::new(HashMap::new()),
            requests: Mutex::new(HashMap::new()),
            deferred: Mutex::new(HashMap::new()),
        })
    }

    /// Send a request; the answer arrives on the returned channel.
    pub fn request(&self, method: &str, params: Value) -> oneshot::Receiver<Result<Value, String>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            // Hover/completion tasks are replaced while typing. Their receivers
            // disappear even if the server never replies; retain only live work.
            p.retain(|_, sender| !sender.is_canceled());
            p.insert(id, tx);
        }
        (self.send)(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        rx
    }

    pub fn notify(&self, method: &str, params: Value) {
        (self.send)(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    pub fn on_notification(&self, method: &str, handler: impl Fn(Value) + std::marker::Send + Sync + 'static) {
        if let Ok(mut n) = self.notifications.lock() {
            n.entry(method.to_owned()).or_default().push(Box::new(handler));
        }
    }

    /// Answer a request *from* the server (e.g. workspace/configuration) right away.
    pub fn on_request(&self, method: &str, handler: impl Fn(Value) -> Value + std::marker::Send + Sync + 'static) {
        if let Ok(mut r) = self.requests.lock() {
            r.insert(method.to_owned(), Box::new(handler));
        }
    }

    /// A request the app answers later with `respond(id, result)` (e.g. workspace/applyEdit).
    pub fn on_request_deferred(
        &self,
        method: &str,
        handler: impl Fn(Value, Value) + std::marker::Send + Sync + 'static,
    ) {
        if let Ok(mut d) = self.deferred.lock() {
            d.insert(method.to_owned(), Box::new(handler));
        }
    }

    pub fn respond(&self, id: Value, result: Value) {
        (self.send)(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    pub fn handle_message(&self, msg: Value) {
        let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = msg.get("id").cloned().filter(|i| !i.is_null());
        match (method, id) {
            // A response to one of our requests.
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else { return };
                let Some(tx) = self.pending.lock().ok().and_then(|mut p| p.remove(&id)) else { return };
                let result = match msg.get("error") {
                    Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("error").to_owned()),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(result);
            }
            // A request from the server: always answer, or it may wait forever.
            (Some(method), Some(id)) => {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                if let Ok(d) = self.deferred.lock() {
                    if let Some(handler) = d.get(&method) {
                        handler(id, params);
                        return;
                    }
                }
                let result =
                    self.requests.lock().ok().and_then(|r| r.get(&method).map(|h| h(params))).unwrap_or(Value::Null);
                self.respond(id, result);
            }
            (Some(method), None) => {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                if let Ok(n) = self.notifications.lock() {
                    for h in n.get(&method).into_iter().flatten() {
                        h(params.clone());
                    }
                }
            }
            (None, None) => {}
        }
    }

    /// Fail everything still waiting (the server stopped or restarted).
    pub fn dispose(&self, reason: &str) {
        if let Ok(mut p) = self.pending.lock() {
            for (_, tx) in p.drain() {
                let _ = tx.send(Err(reason.to_owned()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Arc<LspClient>, Arc<Mutex<Vec<Value>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let s = sent.clone();
        let client = LspClient::new(Arc::new(move |m| s.lock().expect("lock").push(m)));
        (client, sent)
    }

    #[test]
    fn abandoned_requests_do_not_accumulate_while_a_server_is_unresponsive() {
        let (client, _) = setup();
        let live = client.request("keep", Value::Null);
        for _ in 0..1000 {
            drop(client.request("cancelled-hover", Value::Null));
        }
        let next = client.request("next", Value::Null);
        assert_eq!(client.pending.lock().unwrap().len(), 2);
        client.handle_message(json!({"id": 1, "result": "still live"}));
        assert_eq!(futures::executor::block_on(live).unwrap().unwrap(), json!("still live"));
        drop(next);
    }

    #[test]
    fn requests_get_increasing_ids_and_their_matching_responses() {
        let (client, sent) = setup();
        let a = client.request("a", json!({ "x": 1 }));
        let b = client.request("b", json!({}));
        assert_eq!(
            *sent.lock().expect("lock"),
            vec![
                json!({ "jsonrpc": "2.0", "id": 1, "method": "a", "params": { "x": 1 } }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "b", "params": {} }),
            ]
        );
        client.handle_message(json!({ "jsonrpc": "2.0", "id": 2, "result": "B" }));
        client.handle_message(json!({ "jsonrpc": "2.0", "id": 1, "result": "A" }));
        assert_eq!(futures::executor::block_on(a), Ok(Ok(json!("A"))));
        assert_eq!(futures::executor::block_on(b), Ok(Ok(json!("B"))));
    }

    #[test]
    fn errors_fail_the_request() {
        let (client, _) = setup();
        let p = client.request("x", json!({}));
        client.handle_message(json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": "nope" } }));
        assert_eq!(futures::executor::block_on(p), Ok(Err("nope".to_owned())));
    }

    #[test]
    fn notifications_have_no_id_and_reach_listeners() {
        let (client, sent) = setup();
        client.notify("initialized", json!({}));
        assert_eq!(sent.lock().expect("lock")[0], json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        client.on_notification("textDocument/publishDiagnostics", move |p| g.lock().expect("lock").push(p));
        client.handle_message(
            json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": "u" } }),
        );
        client.handle_message(json!({ "jsonrpc": "2.0", "method": "other", "params": {} }));
        assert_eq!(*got.lock().expect("lock"), vec![json!({ "uri": "u" })]);
    }

    #[test]
    fn answers_server_requests_with_a_handler_or_null() {
        let (client, sent) = setup();
        client.on_request("workspace/configuration", |p| {
            Value::Array(
                p["items"].as_array().map(|a| a.iter().map(|_| json!({ "ok": true })).collect()).unwrap_or_default(),
            )
        });
        client.handle_message(json!({ "jsonrpc": "2.0", "id": "srv-1", "method": "workspace/configuration", "params": { "items": [{}, {}] } }));
        client.handle_message(
            json!({ "jsonrpc": "2.0", "id": "srv-2", "method": "client/registerCapability", "params": {} }),
        );
        assert_eq!(
            *sent.lock().expect("lock"),
            vec![
                json!({ "jsonrpc": "2.0", "id": "srv-1", "result": [{ "ok": true }, { "ok": true }] }),
                json!({ "jsonrpc": "2.0", "id": "srv-2", "result": null }),
            ]
        );
    }

    #[test]
    fn deferred_requests_are_answered_later() {
        let (client, sent) = setup();
        let got = Arc::new(Mutex::new(None));
        let g = got.clone();
        client.on_request_deferred("workspace/applyEdit", move |id, params| {
            *g.lock().expect("lock") = Some((id, params))
        });
        client.handle_message(
            json!({ "jsonrpc": "2.0", "id": 7, "method": "workspace/applyEdit", "params": { "edit": {} } }),
        );
        assert!(sent.lock().expect("lock").is_empty());
        let (id, _) = got.lock().expect("lock").clone().expect("asked");
        client.respond(id, json!({ "applied": true }));
        assert_eq!(sent.lock().expect("lock")[0], json!({ "jsonrpc": "2.0", "id": 7, "result": { "applied": true } }));
    }

    #[test]
    fn disposing_fails_pending_requests() {
        let (client, _) = setup();
        let p = client.request("slow", json!({}));
        client.dispose("server exited");
        assert_eq!(futures::executor::block_on(p), Ok(Err("server exited".to_owned())));
    }
}
