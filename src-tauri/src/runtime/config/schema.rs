use crate::runtime::permissions::{Permission, PermissionState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub schema_version: u32,
    pub background_enabled: bool,
    pub launch_at_login: bool,
    pub permission_states: BTreeMap<Permission, PermissionState>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            background_enabled: false,
            launch_at_login: false,
            permission_states: BTreeMap::new(),
        }
    }
}
