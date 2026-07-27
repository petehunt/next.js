//! The server-island registry.
//!
//! `#[next::island]` registers each island at link time via `inventory`, so the
//! Node side can enumerate islands (for the build manifest and slot validation)
//! and render one by id without any generated dispatch table.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::fragment::Fragment;
use crate::ts::TypeDecl;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Rendering an island is always fallible: a panic at the FFI boundary would
/// take down the process, so `#[next::island]` catches and converts.
pub type RenderFn = fn(serde_json::Value) -> BoxFuture<Result<Fragment, IslandError>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IslandError {
    pub island: String,
    pub message: String,
}

impl std::fmt::Display for IslandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "island `{}` failed: {}", self.island, self.message)
    }
}

impl std::error::Error for IslandError {}

/// A slot an island declares it will emit, via `#[next::slot(...)]`.
///
/// Not serializable: `props_ts` is a function pointer so the TypeScript text is
/// produced on demand rather than baked into every binary as a string. The
/// serializable projection is [`SlotManifestEntry`].
pub struct SlotDecl {
    pub name: &'static str,
    /// TypeScript type of the props Rust supplies to whatever fills this slot.
    pub props_ts: fn() -> String,
}

/// One registered island.
pub struct IslandDef {
    pub id: &'static str,
    pub slots: &'static [SlotDecl],
    pub props_ts: fn() -> String,
    pub decls: fn(&mut BTreeMap<String, TypeDecl>),
    pub render: RenderFn,
}

inventory::collect!(IslandDef);

/// The build-time manifest: what the JS side needs to generate descriptors,
/// type slot props, and validate `<Island>` usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IslandManifestEntry {
    pub id: String,
    pub props_ts: String,
    pub slots: Vec<SlotManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotManifestEntry {
    pub name: String,
    pub props_ts: String,
}

pub fn islands() -> Vec<&'static IslandDef> {
    let mut v: Vec<_> = inventory::iter::<IslandDef>.into_iter().collect();
    v.sort_by_key(|i| i.id);
    v
}

pub fn manifest() -> Vec<IslandManifestEntry> {
    islands()
        .into_iter()
        .map(|i| IslandManifestEntry {
            id: i.id.to_owned(),
            props_ts: (i.props_ts)(),
            slots: i
                .slots
                .iter()
                .map(|s| SlotManifestEntry { name: s.name.to_owned(), props_ts: (s.props_ts)() })
                .collect(),
        })
        .collect()
}

/// Every named type declaration reachable from any island's props.
pub fn type_decls() -> BTreeMap<String, TypeDecl> {
    let mut out = BTreeMap::new();
    for i in islands() {
        (i.decls)(&mut out);
    }
    out
}

pub fn find(id: &str) -> Option<&'static IslandDef> {
    islands().into_iter().find(|i| i.id == id)
}

/// Renders one island. Unknown ids are an error rather than a panic, because
/// the id arrives from the JS side and a stale manifest should not abort the
/// process.
pub async fn render(id: &str, props: serde_json::Value) -> Result<Fragment, IslandError> {
    match find(id) {
        Some(def) => (def.render)(props).await,
        None => Err(IslandError {
            island: id.to_owned(),
            message: "no island registered with this id".to_owned(),
        }),
    }
}
