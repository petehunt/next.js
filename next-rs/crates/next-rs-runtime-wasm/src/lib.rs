//! The browser WASM bridge (spec §3.7, §10, §12, §89).
//!
//! WASM is a supporting capability, not a component system: it is an execution
//! target for explicitly browser-compatible Rust functions (spec §3.7). This crate
//! is the dispatch layer the generated `wasm-bindgen` glue calls into, and it holds
//! the invariant that makes §11 safe at runtime as well as at build time:
//!
//! > **A [`WasmBridge`] refuses to be constructed with a server-only export.**
//!
//! Build-time checking catches the import; this catches a mis-generated bundle.

#![deny(missing_debug_implementations)]

use std::sync::Arc;

use next_rs_core::{Error, ExportRegistry, ExportTarget, Result};

/// Dispatches browser calls into `#[export(client)]` functions.
#[derive(Debug, Clone)]
pub struct WasmBridge {
    exports: Arc<ExportRegistry>,
}

impl WasmBridge {
    /// Builds a bridge over the browser-compatible exports of `registry`.
    ///
    /// Server-only exports are dropped rather than rejected, because the registry
    /// legitimately contains both; use [`WasmBridge::strict`] when the registry is
    /// supposed to be client-only already.
    pub fn new(registry: &ExportRegistry) -> Result<Self> {
        let mut client_only = ExportRegistry::new();
        for export in registry.client_exports() {
            client_only.register(export.clone())?;
        }
        Ok(Self {
            exports: Arc::new(client_only),
        })
    }

    /// Builds a bridge, failing if `registry` contains anything server-only.
    ///
    /// This is what the generated WASM entry point uses: if a server-only export
    /// ever reached the browser bundle, that is a build bug worth failing loudly
    /// rather than silently dropping.
    pub fn strict(registry: &ExportRegistry) -> Result<Self> {
        let server_only: Vec<&str> = registry
            .names()
            .filter(|name| {
                registry
                    .get(name)
                    .is_some_and(|export| export.target() != ExportTarget::Client)
            })
            .collect();
        if !server_only.is_empty() {
            return Err(Error::internal(format!(
                "server-only export(s) reached the browser bundle: {}",
                server_only.join(", ")
            )));
        }
        Self::new(registry)
    }

    /// Names the browser may call.
    pub fn names(&self) -> Vec<String> {
        self.exports.names().map(str::to_owned).collect()
    }

    pub fn len(&self) -> usize {
        self.exports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }

    /// Calls an export with a JSON array of arguments.
    ///
    /// Returns a JSON envelope so the generated glue can resolve or reject a
    /// JavaScript promise without needing a Rust panic hook.
    pub async fn call_json(&self, name: &str, args_json: &str) -> String {
        match self.call(name, args_json).await {
            Ok(value) => serde_json::json!({ "ok": value }).to_string(),
            Err(error) => serde_json::json!({
                "error": { "code": error.code(), "message": error.public_message() }
            })
            .to_string(),
        }
    }

    /// Calls an export, surfacing errors as `Result` for Rust callers.
    pub async fn call(&self, name: &str, args_json: &str) -> Result<serde_json::Value> {
        // A server-only name must read as "not here", never as "not allowed":
        // the browser bundle should not learn that a server export exists.
        let args = match serde_json::from_str::<serde_json::Value>(args_json.trim()) {
            Ok(serde_json::Value::Array(values)) => values,
            Ok(_) => return Err(Error::bad_request("export arguments must be a JSON array")),
            Err(_) if args_json.trim().is_empty() => Vec::new(),
            Err(error) => {
                return Err(Error::bad_request(format!(
                    "export arguments are not valid JSON: {error}"
                )));
            }
        };
        self.exports.call(name, args).await
    }
}

#[cfg(test)]
mod tests {
    use next_rs_core::ExportRegistration;
    use serde_json::json;

    use super::*;

    fn registration(name: &str, target: ExportTarget) -> ExportRegistration {
        ExportRegistration::new(
            name,
            name,
            1,
            false,
            target,
            Arc::new(|args| {
                Box::pin(async move {
                    let query = args[0].as_str().unwrap_or_default().to_owned();
                    Ok(json!(format!("matched:{query}")))
                })
            }),
        )
    }

    fn mixed_registry() -> ExportRegistry {
        ExportRegistry::new()
            .with(registration("fuzzy_search", ExportTarget::Client))
            .unwrap()
            .with(registration("private_search", ExportTarget::Server))
            .unwrap()
    }

    #[tokio::test]
    async fn exposes_only_browser_compatible_exports() {
        let bridge = WasmBridge::new(&mixed_registry()).unwrap();
        assert_eq!(bridge.names(), vec!["fuzzy_search"]);
        assert_eq!(bridge.len(), 1);
        assert!(!bridge.is_empty());

        let out = bridge.call_json("fuzzy_search", r#"["ru"]"#).await;
        assert_eq!(out, r#"{"ok":"matched:ru"}"#);
    }

    #[tokio::test]
    async fn a_server_only_export_is_simply_not_there() {
        let bridge = WasmBridge::new(&mixed_registry()).unwrap();
        let out = bridge.call_json("private_search", r#"["x"]"#).await;
        assert!(out.contains("NOT_FOUND"));
        // The browser must not be told that a server export exists.
        assert!(!out.contains("server-only"));
        assert!(!out.contains("forbidden"));
    }

    #[test]
    fn strict_mode_refuses_a_bundle_containing_server_exports() {
        let error = WasmBridge::strict(&mixed_registry()).unwrap_err();
        assert!(error.message().contains("private_search"));
        assert!(error.message().contains("reached the browser bundle"));
    }

    #[test]
    fn strict_mode_accepts_a_client_only_registry() {
        let registry = ExportRegistry::new()
            .with(registration("fuzzy_search", ExportTarget::Client))
            .unwrap();
        assert_eq!(WasmBridge::strict(&registry).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejects_malformed_arguments() {
        let bridge = WasmBridge::new(&mixed_registry()).unwrap();
        assert!(
            bridge
                .call_json("fuzzy_search", "not json")
                .await
                .contains("not valid JSON")
        );
        assert!(
            bridge
                .call_json("fuzzy_search", "42")
                .await
                .contains("must be a JSON array")
        );
    }

    #[tokio::test]
    async fn an_empty_argument_list_is_accepted() {
        let registry = ExportRegistry::new()
            .with(ExportRegistration::new(
                "version",
                "version",
                0,
                false,
                ExportTarget::Client,
                Arc::new(|_| Box::pin(async { Ok(json!("1.0")) })),
            ))
            .unwrap();
        let bridge = WasmBridge::new(&registry).unwrap();
        assert_eq!(bridge.call_json("version", "").await, r#"{"ok":"1.0"}"#);
    }

    #[test]
    fn an_empty_registry_produces_an_empty_bridge() {
        let bridge = WasmBridge::new(&ExportRegistry::new()).unwrap();
        assert!(bridge.is_empty());
        assert!(WasmBridge::strict(&ExportRegistry::new()).is_ok());
    }
}
