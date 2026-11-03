use crate::runtime::permissions::{Permission, PermissionState};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

use super::validation::config_from_value;

/// The schema version written by this build and the newest one it can read.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Field names this build understands at the top level of `runtime.json`.
pub(crate) const KNOWN_FIELDS: [&str; 4] = [
    "schema_version",
    "background_enabled",
    "launch_at_login",
    "permission_states",
];

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub schema_version: u32,
    pub background_enabled: bool,
    pub launch_at_login: bool,
    pub permission_states: BTreeMap<Permission, PermissionState>,
    /// Entries under `permission_states` whose names this build does not
    /// recognise, kept exactly as read so that saving never discards them.
    /// They are never interpreted, so they can never authorize anything.
    pub unknown_permissions: BTreeMap<String, Value>,
    /// Top-level fields this build does not recognise, kept exactly as read
    /// so that saving never discards them.
    pub unknown_fields: BTreeMap<String, Value>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            background_enabled: false,
            launch_at_login: false,
            permission_states: BTreeMap::new(),
            unknown_permissions: BTreeMap::new(),
            unknown_fields: BTreeMap::new(),
        }
    }
}

impl RuntimeConfig {
    /// Known and unknown permission entries, as they appear in the file.
    fn permission_entries(&self) -> Result<BTreeMap<String, Value>, serde_json::Error> {
        let mut entries = self.unknown_permissions.clone();

        for (permission, state) in &self.permission_states {
            let name = match serde_json::to_value(permission)? {
                Value::String(name) => name,
                other => other.to_string(),
            };

            entries.insert(name, serde_json::to_value(state)?);
        }

        Ok(entries)
    }
}

impl Serialize for RuntimeConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let permission_entries = self
            .permission_entries()
            .map_err(serde::ser::Error::custom)?;

        let mut map = serializer.serialize_map(None)?;

        map.serialize_entry("schema_version", &self.schema_version)?;
        map.serialize_entry("background_enabled", &self.background_enabled)?;
        map.serialize_entry("launch_at_login", &self.launch_at_login)?;
        map.serialize_entry("permission_states", &permission_entries)?;

        for (name, value) in &self.unknown_fields {
            // A known name here would write the field twice.
            if !KNOWN_FIELDS.contains(&name.as_str()) {
                map.serialize_entry(name, value)?;
            }
        }

        map.end()
    }
}

impl<'de> Deserialize<'de> for RuntimeConfig {
    /// Uses the same validation as loading from disk, so a configuration that
    /// cannot be interpreted safely is rejected here as well.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;

        config_from_value(value)
            .map(|parsed| parsed.config)
            .map_err(serde::de::Error::custom)
    }
}
