//! The Rust half of the example application (spec §6).

pub mod react;

use next_rs::prelude::*;
use serde::{Deserialize, Serialize};

/// A transparent Rust export, imported from TypeScript as `normalizeSlug`
/// (spec §7).
#[export]
pub fn normalize_slug(value: String) -> String {
    value.trim().to_lowercase().replace(' ', "-")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchInput {
    pub query: String,
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
}

/// Server-only by default (spec §11): importing this from a Client Component
/// fails the build.
#[export]
pub async fn search(input: SearchInput) -> Result<Vec<SearchResult>> {
    Ok((0..input.limit)
        .map(|index| SearchResult {
            id: index.to_string(),
            title: format!("{} #{index}", input.query),
        })
        .collect())
}

/// Explicitly browser-compatible, so it also compiles to WASM (spec §10).
#[export(client)]
pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> {
    candidates
        .into_iter()
        .filter(|candidate| candidate.contains(&query))
        .collect()
}
