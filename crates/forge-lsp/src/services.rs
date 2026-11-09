//! Multiple language servers side by side, routing by the documents they own.
//! Replaces `src/renderer/lsp/services.ts`.
use crate::navigation::WorkspaceSymbol;
use crate::server::{merge_diagnostics, path_key, Diagnostic, LspServer};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Default)]
pub struct LanguageServices {
    servers: BTreeMap<String, Arc<LspServer>>,
}

impl LanguageServices {
    pub fn insert(&mut self, server: Arc<LspServer>) {
        self.servers.insert(server.config.id.into(), server);
    }
    pub fn remove(&mut self, id: &str) {
        if let Some(server) = self.servers.remove(id) {
            server.stopped();
        }
    }
    pub fn clear(&mut self) {
        for server in self.servers.values() {
            server.stopped();
        }
        self.servers.clear();
    }
    pub fn server(&self, id: &str) -> Option<Arc<LspServer>> {
        self.servers.get(id).cloned()
    }
    pub fn for_path(&self, path: &str) -> Option<Arc<LspServer>> {
        self.servers.values().find(|s| s.knows(path)).cloned()
    }
    pub fn all(&self) -> Vec<Arc<LspServer>> {
        self.servers.values().cloned().collect()
    }
    pub fn sync_docs(&self, docs: &[(String, String, String)]) {
        for server in self.servers.values() {
            server.sync_docs(docs);
        }
    }
    pub fn changed(&self, path: &str) {
        for server in self.servers.values() {
            server.doc_changed(path);
        }
    }
    pub fn flush(&self, text: &dyn Fn(&str) -> Option<String>) {
        for server in self.servers.values() {
            if server.has_pending() {
                server.flush(text);
            }
        }
    }
    pub fn did_save(&self, path: &str, text: &dyn Fn(&str) -> Option<String>) {
        for server in self.servers.values() {
            if server.knows(path) {
                server.did_save(path, text);
            }
        }
    }
    pub fn diagnostics_for(&self, path: &str) -> Vec<Diagnostic> {
        self.servers.values().flat_map(|s| s.diagnostics_for(path)).collect()
    }
    pub fn all_diagnostics(&self) -> Vec<(String, Vec<Diagnostic>)> {
        let mut files: BTreeMap<String, (String, Vec<Diagnostic>)> = BTreeMap::new();
        for server in self.servers.values() {
            for (path, list) in server.all_diagnostics() {
                let entry = files.entry(path_key(&path)).or_insert_with(|| (path, Vec::new()));
                entry.1 = merge_diagnostics(&entry.1, &list);
            }
        }
        files.into_values().collect()
    }
    pub async fn workspace_symbols(servers: Vec<Arc<LspServer>>, query: &str) -> Vec<WorkspaceSymbol> {
        futures::future::join_all(servers.iter().map(|s| s.workspace_symbols(query)))
            .await
            .into_iter()
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{client::LspClient, server::ServerConfig};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    fn server(config: ServerConfig) -> Arc<LspServer> {
        let slot: Arc<Mutex<Option<Arc<LspClient>>>> = Arc::new(Mutex::new(None));
        let reply = slot.clone();
        let client = LspClient::new(Arc::new(move |m| {
            if m["method"] == "initialize" {
                let c = reply.lock().unwrap().clone().unwrap();
                c.handle_message(json!({"jsonrpc":"2.0","id":m["id"],"result":{"capabilities":{}}}));
            }
        }));
        *slot.lock().unwrap() = Some(client.clone());
        let server = LspServer::new(client, config, || {});
        futures::executor::block_on(server.start("/p", json!({}))).unwrap();
        server
    }
    #[test]
    fn two_servers_keep_their_own_documents_and_diagnostics() {
        let py = server(crate::server::python_config());
        let json = server(ServerConfig {
            id: "json-test",
            name: "JSON",
            languages: &["json"],
            configuration: |_, _| Value::Null,
        });
        let mut pool = LanguageServices::default();
        assert!(py.rename_error("bad-name").is_some());
        assert!(json.rename_error("$validForAnotherLanguage").is_none());
        pool.insert(py.clone());
        pool.insert(json.clone());
        pool.sync_docs(&[
            ("/p/a.py".into(), "python".into(), "x=1".into()),
            ("/p/a.json".into(), "json".into(), "{}".into()),
        ]);
        assert!(Arc::ptr_eq(&pool.for_path("/p/a.py").unwrap(), &py));
        assert!(Arc::ptr_eq(&pool.for_path("/p/a.json").unwrap(), &json));
        assert!(!json.knows("/p/a.py"));
        for (server, path) in [(&py, "/p/a.py"), (&json, "/p/a.json")] {
            server.client.handle_message(json!({"method":"textDocument/publishDiagnostics","params":{"uri":crate::uri::path_to_uri(path),"diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"problem","severity":1}]}}));
        }
        assert_eq!(pool.all_diagnostics().len(), 2);
        pool.remove("pyright");
        assert!(pool.for_path("/p/a.py").is_none());
        assert!(pool.for_path("/p/a.json").is_some());
        assert_eq!(pool.all_diagnostics().len(), 1);
    }
}
