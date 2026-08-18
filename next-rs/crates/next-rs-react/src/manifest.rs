use serde::{Deserialize, Serialize};

/// `.next-rs/manifests/react-components.json` (spec §83).
///
/// Written by the build from `next-rs.components.ts` (spec §24) and read by the
/// browser runtime and the React renderer service to resolve a component ID to a
/// module.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentManifest {
    pub build_id: String,
    pub components: Vec<ComponentManifestEntry>,
}

impl ComponentManifest {
    pub fn get(&self, id: &str) -> Option<&ComponentManifestEntry> {
        self.components.entry_for(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// Returns the IDs registered more than once.
    ///
    /// Duplicate component IDs are a build error: Rust bindings would be
    /// ambiguous, and the browser could mount the wrong module.
    pub fn duplicate_ids(&self) -> Vec<&str> {
        duplicates(self.components.iter().map(|entry| entry.id.as_str()))
    }
}

/// One registered React Client Component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentManifestEntry {
    /// The name used from Rust, e.g. `Dashboard`.
    pub id: String,
    /// Module specifier as written in `next-rs.components.ts`.
    pub module: String,
    /// Export name within the module.
    pub export: String,
    /// Client bundle chunk that provides the component, when known.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chunk: Option<String>,
}

/// `.next-rs/manifests/react-loaders.json` (spec §83).
///
/// The authoritative list of loaders `/__next_rs/react` may invoke (spec §65).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderManifest {
    pub build_id: String,
    pub protocol_version: u16,
    pub loaders: Vec<LoaderManifestEntry>,
}

impl LoaderManifest {
    pub fn get(&self, id: &str) -> Option<&LoaderManifestEntry> {
        self.loaders.iter().find(|entry| entry.id == id)
    }

    pub fn duplicate_ids(&self) -> Vec<&str> {
        duplicates(self.loaders.iter().map(|entry| entry.id.as_str()))
    }

    /// Loader entries that name a component the component manifest does not
    /// contain — a build-time inconsistency worth failing on.
    pub fn unresolved_components<'a>(&'a self, components: &ComponentManifest) -> Vec<&'a str> {
        self.loaders
            .iter()
            .filter(|entry| !components.contains(&entry.component))
            .map(|entry| entry.component.as_str())
            .collect()
    }
}

/// One registered `#[react_component]` loader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderManifestEntry {
    /// Loader ID referenced by markers and tokens.
    pub id: String,
    /// The component this loader produces props for.
    pub component: String,
    /// Argument count after `RenderContext`.
    pub arity: usize,
}

/// Small helper trait so `ComponentManifest::get` reads naturally.
trait EntryLookup {
    fn entry_for(&self, id: &str) -> Option<&ComponentManifestEntry>;
}

impl EntryLookup for Vec<ComponentManifestEntry> {
    fn entry_for(&self, id: &str) -> Option<&ComponentManifestEntry> {
        self.iter().find(|entry| entry.id == id)
    }
}

fn duplicates<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicated = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            duplicated.insert(id);
        }
    }
    duplicated.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(id: &str) -> ComponentManifestEntry {
        ComponentManifestEntry {
            id: id.to_owned(),
            module: format!("@/components/{id}"),
            export: "default".to_owned(),
            chunk: None,
        }
    }

    fn loader(id: &str, component: &str) -> LoaderManifestEntry {
        LoaderManifestEntry {
            id: id.to_owned(),
            component: component.to_owned(),
            arity: 1,
        }
    }

    #[test]
    fn looks_up_components_and_loaders() {
        let components = ComponentManifest {
            build_id: "b1".to_owned(),
            components: vec![component("Dashboard")],
        };
        assert!(components.contains("Dashboard"));
        assert!(!components.contains("Missing"));
        assert_eq!(
            components.get("Dashboard").unwrap().module,
            "@/components/Dashboard"
        );

        let loaders = LoaderManifest {
            build_id: "b1".to_owned(),
            protocol_version: 1,
            loaders: vec![loader("dashboard", "Dashboard")],
        };
        assert_eq!(loaders.get("dashboard").unwrap().arity, 1);
        assert!(loaders.get("nope").is_none());
    }

    #[test]
    fn detects_duplicates() {
        let components = ComponentManifest {
            build_id: "b1".to_owned(),
            components: vec![component("Dashboard"), component("Dashboard")],
        };
        assert_eq!(components.duplicate_ids(), vec!["Dashboard"]);

        let loaders = LoaderManifest {
            build_id: "b1".to_owned(),
            protocol_version: 1,
            loaders: vec![
                loader("dashboard", "Dashboard"),
                loader("dashboard", "Other"),
            ],
        };
        assert_eq!(loaders.duplicate_ids(), vec!["dashboard"]);
    }

    #[test]
    fn detects_loaders_pointing_at_unregistered_components() {
        let components = ComponentManifest {
            build_id: "b1".to_owned(),
            components: vec![component("Dashboard")],
        };
        let loaders = LoaderManifest {
            build_id: "b1".to_owned(),
            protocol_version: 1,
            loaders: vec![
                loader("dashboard", "Dashboard"),
                loader("ghost", "NotRegistered"),
            ],
        };
        assert_eq!(
            loaders.unresolved_components(&components),
            vec!["NotRegistered"]
        );
    }

    #[test]
    fn serialises_as_camel_case_json() {
        let manifest = LoaderManifest {
            build_id: "b1".to_owned(),
            protocol_version: 1,
            loaders: vec![loader("dashboard", "Dashboard")],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("\"buildId\":\"b1\""));
        assert!(json.contains("\"protocolVersion\":1"));

        let round_tripped: LoaderManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, manifest);
    }

    #[test]
    fn component_chunks_are_optional() {
        let json = r#"{"buildId":"b1","components":[{"id":"D","module":"m","export":"default"}]}"#;
        let manifest: ComponentManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.components[0].chunk, None);
    }
}
