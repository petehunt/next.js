use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, OnceLock},
};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where an `#[export]` may run (spec §10, §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportTarget {
    /// The default: server only, reachable natively or through N-API.
    Server,
    /// `#[export(client)]`: additionally compiled to browser WASM.
    Client,
}

impl ExportTarget {
    pub const fn is_browser_compatible(self) -> bool {
        matches!(self, Self::Client)
    }
}

/// The erased invocation shim generated for an export.
type ExportInvoke = Arc<
    dyn Fn(Vec<serde_json::Value>) -> BoxFuture<'static, Result<serde_json::Value>> + Send + Sync,
>;

/// A Rust function exposed to TypeScript (spec §7).
#[derive(Clone)]
pub struct ExportRegistration {
    name: String,
    js_name: String,
    arity: usize,
    is_async: bool,
    target: ExportTarget,
    invoke: ExportInvoke,
}

impl ExportRegistration {
    pub fn new(
        name: impl Into<String>,
        js_name: impl Into<String>,
        arity: usize,
        is_async: bool,
        target: ExportTarget,
        invoke: ExportInvoke,
    ) -> Self {
        Self {
            name: name.into(),
            js_name: js_name.into(),
            arity,
            is_async,
            target,
            invoke,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The identifier TypeScript imports, e.g. `normalizeSlug`.
    pub fn js_name(&self) -> &str {
        &self.js_name
    }

    pub fn arity(&self) -> usize {
        self.arity
    }

    pub fn is_async(&self) -> bool {
        self.is_async
    }

    pub fn target(&self) -> ExportTarget {
        self.target
    }

    /// Calls the export with JSON arguments, checking arity first.
    pub async fn call(&self, args: Vec<serde_json::Value>) -> Result<serde_json::Value> {
        if args.len() != self.arity {
            return Err(Error::bad_request(format!(
                "export `{}` expects {} argument(s), got {}",
                self.name,
                self.arity,
                args.len()
            )));
        }
        (self.invoke)(args).await
    }
}

impl fmt::Debug for ExportRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExportRegistration")
            .field("name", &self.name)
            .field("js_name", &self.js_name)
            .field("arity", &self.arity)
            .field("is_async", &self.is_async)
            .field("target", &self.target)
            .finish()
    }
}

/// The set of `#[export]` functions a build exposes.
///
/// The N-API and WASM bridges dispatch through this registry rather than through
/// arbitrary symbol lookup.
#[derive(Default, Clone)]
pub struct ExportRegistry {
    exports: BTreeMap<String, ExportRegistration>,
}

static GLOBAL_EXPORTS: OnceLock<ExportRegistry> = OnceLock::new();

impl ExportRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, registration: ExportRegistration) -> Result<()> {
        let name = registration.name().to_owned();
        if self.exports.contains_key(&name) {
            return Err(Error::internal(format!("duplicate export `{name}`")));
        }
        self.exports.insert(name, registration);
        Ok(())
    }

    pub fn with(mut self, registration: ExportRegistration) -> Result<Self> {
        self.register(registration)?;
        Ok(self)
    }

    pub fn get(&self, name: &str) -> Option<&ExportRegistration> {
        self.exports.get(name)
    }

    /// Looks up by the TypeScript-facing name.
    pub fn get_by_js_name(&self, js_name: &str) -> Option<&ExportRegistration> {
        self.exports
            .values()
            .find(|export| export.js_name() == js_name)
    }

    pub fn len(&self) -> usize {
        self.exports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.exports.keys().map(String::as_str)
    }

    /// Invokes an export by name.
    pub async fn call(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        self.get(name)
            .ok_or_else(|| Error::not_found(format!("unknown export `{name}`")))?
            .call(args)
            .await
    }

    /// Exports compiled to browser WASM (spec §10).
    pub fn client_exports(&self) -> impl Iterator<Item = &ExportRegistration> {
        self.exports
            .values()
            .filter(|export| export.target().is_browser_compatible())
    }

    pub fn manifest(&self, build_id: impl Into<String>) -> ExportManifest {
        ExportManifest {
            build_id: build_id.into(),
            exports: self
                .exports
                .values()
                .map(|export| ExportManifestEntry {
                    name: export.name().to_owned(),
                    js_name: export.js_name().to_owned(),
                    arity: export.arity(),
                    is_async: export.is_async(),
                    target: export.target(),
                })
                .collect(),
        }
    }

    pub fn install_global(self) -> Result<()> {
        GLOBAL_EXPORTS
            .set(self)
            .map_err(|_| Error::internal("a global export registry is already installed"))
    }

    pub fn global() -> Option<&'static Self> {
        GLOBAL_EXPORTS.get()
    }
}

impl fmt::Debug for ExportRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExportRegistry")
            .field("exports", &self.exports.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// `.next-rs/manifests/rust-exports.json`, the input to TypeScript binding
/// generation (spec §82 step 8).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifest {
    pub build_id: String,
    pub exports: Vec<ExportManifestEntry>,
}

/// One exported function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifestEntry {
    pub name: String,
    pub js_name: String,
    pub arity: usize,
    pub is_async: bool,
    pub target: ExportTarget,
}

/// Decodes argument `index` for a generated export or loader shim.
///
/// Kept as a function rather than inlined into the macro so the error message is
/// consistent and the generated code stays small.
pub fn decode_arg<T: serde::de::DeserializeOwned>(
    args: &[serde_json::Value],
    index: usize,
    name: &str,
) -> Result<T> {
    let value = args
        .get(index)
        .ok_or_else(|| Error::bad_request(format!("missing argument {index} (`{name}`)")))?;
    serde_json::from_value(value.clone()).map_err(|error| {
        Error::bad_request(format!(
            "argument {index} (`{name}`) has the wrong type: {error}"
        ))
    })
}

/// Encodes a return value for a generated shim.
pub fn encode_result<T: serde::Serialize>(value: T) -> Result<serde_json::Value> {
    serde_json::to_value(value)
        .map_err(|error| Error::internal(format!("return value is not serialisable: {error}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn registration(name: &str, arity: usize, target: ExportTarget) -> ExportRegistration {
        ExportRegistration::new(
            name,
            name,
            arity,
            false,
            target,
            Arc::new(|args| Box::pin(async move { Ok(json!(args)) })),
        )
    }

    #[tokio::test]
    async fn calls_an_export() {
        let registry = ExportRegistry::new()
            .with(registration("normalize_slug", 1, ExportTarget::Server))
            .unwrap();
        let result = registry
            .call("normalize_slug", vec![json!("Hello World")])
            .await
            .unwrap();
        assert_eq!(result, json!(["Hello World"]));
    }

    #[tokio::test]
    async fn unknown_exports_are_not_callable() {
        let registry = ExportRegistry::new();
        let error = registry.call("anything", vec![]).await.unwrap_err();
        assert_eq!(error.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn arity_is_enforced() {
        let registry = ExportRegistry::new()
            .with(registration("f", 2, ExportTarget::Server))
            .unwrap();
        let error = registry.call("f", vec![json!(1)]).await.unwrap_err();
        assert!(error.message().contains("expects 2 argument(s), got 1"));
    }

    #[test]
    fn rejects_duplicate_exports() {
        let mut registry = ExportRegistry::new();
        registry
            .register(registration("f", 0, ExportTarget::Server))
            .unwrap();
        assert!(
            registry
                .register(registration("f", 0, ExportTarget::Server))
                .is_err()
        );
    }

    #[test]
    fn separates_client_exports() {
        let registry = ExportRegistry::new()
            .with(registration("server_only", 0, ExportTarget::Server))
            .unwrap()
            .with(registration("fuzzy_search", 0, ExportTarget::Client))
            .unwrap();
        assert_eq!(registry.client_exports().count(), 1);
        assert_eq!(
            registry.client_exports().next().unwrap().name(),
            "fuzzy_search"
        );
        assert!(!ExportTarget::Server.is_browser_compatible());
    }

    #[test]
    fn looks_up_by_js_name() {
        let registry = ExportRegistry::new()
            .with(ExportRegistration::new(
                "normalize_slug",
                "normalizeSlug",
                1,
                false,
                ExportTarget::Server,
                Arc::new(|_| Box::pin(async { Ok(json!(null)) })),
            ))
            .unwrap();
        assert!(registry.get_by_js_name("normalizeSlug").is_some());
        assert!(registry.get_by_js_name("normalize_slug").is_none());
    }

    #[test]
    fn produces_a_manifest() {
        let registry = ExportRegistry::new()
            .with(registration("fuzzy_search", 2, ExportTarget::Client))
            .unwrap();
        let manifest = registry.manifest("build-1");
        assert_eq!(manifest.exports.len(), 1);
        assert_eq!(manifest.exports[0].target, ExportTarget::Client);

        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("\"jsName\""));
        assert!(json.contains("\"client\""));
        assert_eq!(
            serde_json::from_str::<ExportManifest>(&json).unwrap(),
            manifest
        );
    }

    #[test]
    fn decodes_and_encodes_shim_values() {
        let args = vec![json!("hi"), json!(3)];
        assert_eq!(decode_arg::<String>(&args, 0, "value").unwrap(), "hi");
        assert_eq!(decode_arg::<u32>(&args, 1, "limit").unwrap(), 3);

        let error = decode_arg::<u32>(&args, 0, "value").unwrap_err();
        assert!(error.message().contains("wrong type"));

        let error = decode_arg::<u32>(&args, 9, "missing").unwrap_err();
        assert!(error.message().contains("missing argument 9"));

        assert_eq!(encode_result(vec![1, 2]).unwrap(), json!([1, 2]));
    }

    #[test]
    fn debug_lists_export_names() {
        let registry = ExportRegistry::new()
            .with(registration("f", 0, ExportTarget::Server))
            .unwrap();
        assert!(format!("{registry:?}").contains('f'));
        assert!(format!("{:?}", registration("f", 0, ExportTarget::Server)).contains("arity"));
    }
}
