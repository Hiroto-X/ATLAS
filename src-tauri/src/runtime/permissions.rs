use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    HardwareMetadata,
    ProcessMetadata,
    SoftwareInventory,
    FilesystemMetadata,
    NetworkMetadata,
}

impl Permission {
    /// Every permission, in a stable order.
    pub const ALL: [Permission; 5] = [
        Permission::HardwareMetadata,
        Permission::ProcessMetadata,
        Permission::SoftwareInventory,
        Permission::FilesystemMetadata,
        Permission::NetworkMetadata,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    NotRequested,
    Granted,
    Denied,
}

pub struct PermissionManager {
    permissions: HashMap<Permission, PermissionState>,
}

impl PermissionManager {
    pub fn new() -> Self {
        Self {
            permissions: HashMap::new(),
        }
    }

    /// Creates a manager from previously saved permission decisions.
    ///
    /// Permissions missing from `states` stay `NotRequested`, so nothing is
    /// granted unless a saved decision explicitly says so.
    pub fn from_states(states: &BTreeMap<Permission, PermissionState>) -> Self {
        Self {
            permissions: states
                .iter()
                .map(|(permission, state)| (*permission, *state))
                .collect(),
        }
    }

    pub fn get_state(&self, permission: Permission) -> PermissionState {
        self.permissions
            .get(&permission)
            .copied()
            .unwrap_or(PermissionState::NotRequested)
    }

    pub fn grant(&mut self, permission: Permission) {
        self.permissions
            .insert(permission, PermissionState::Granted);
    }

    pub fn deny(&mut self, permission: Permission) {
        self.permissions.insert(permission, PermissionState::Denied);
    }

    pub fn revoke(&mut self, permission: Permission) {
        self.permissions
            .insert(permission, PermissionState::NotRequested);
    }

    pub fn is_granted(&self, permission: Permission) -> bool {
        self.get_state(permission) == PermissionState::Granted
    }
}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive on purpose: adding a permission breaks this until `ALL` and
    /// this list are updated.
    fn position(permission: Permission) -> usize {
        match permission {
            Permission::HardwareMetadata => 0,
            Permission::ProcessMetadata => 1,
            Permission::SoftwareInventory => 2,
            Permission::FilesystemMetadata => 3,
            Permission::NetworkMetadata => 4,
        }
    }

    #[test]
    fn all_lists_every_permission_once_in_order() {
        for (index, permission) in Permission::ALL.iter().enumerate() {
            assert_eq!(position(*permission), index);
        }
    }

    #[test]
    fn permissions_are_not_requested_by_default() {
        let manager = PermissionManager::new();

        assert_eq!(
            manager.get_state(Permission::HardwareMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn can_grant_permission() {
        let mut manager = PermissionManager::new();

        manager.grant(Permission::HardwareMetadata);

        assert!(manager.is_granted(Permission::HardwareMetadata));
    }

    #[test]
    fn can_deny_permission() {
        let mut manager = PermissionManager::new();

        manager.deny(Permission::ProcessMetadata);

        assert_eq!(
            manager.get_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );

        assert!(!manager.is_granted(Permission::ProcessMetadata));
    }

    #[test]
    fn can_revoke_permission() {
        let mut manager = PermissionManager::new();

        manager.grant(Permission::NetworkMetadata);
        manager.revoke(Permission::NetworkMetadata);

        assert_eq!(
            manager.get_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn permissions_are_independent() {
        let mut manager = PermissionManager::new();

        manager.grant(Permission::HardwareMetadata);

        assert!(manager.is_granted(Permission::HardwareMetadata));

        assert!(!manager.is_granted(Permission::FilesystemMetadata));
    }

    #[test]
    fn restores_saved_permission_states() {
        let mut saved = BTreeMap::new();
        saved.insert(Permission::HardwareMetadata, PermissionState::Granted);
        saved.insert(Permission::NetworkMetadata, PermissionState::Denied);

        let manager = PermissionManager::from_states(&saved);

        assert!(manager.is_granted(Permission::HardwareMetadata));
        assert_eq!(
            manager.get_state(Permission::NetworkMetadata),
            PermissionState::Denied
        );
        assert!(!manager.is_granted(Permission::NetworkMetadata));
    }

    #[test]
    fn permissions_missing_from_saved_states_are_not_requested() {
        let mut saved = BTreeMap::new();
        saved.insert(Permission::HardwareMetadata, PermissionState::Granted);

        let manager = PermissionManager::from_states(&saved);

        assert_eq!(
            manager.get_state(Permission::ProcessMetadata),
            PermissionState::NotRequested
        );
        assert!(!manager.is_granted(Permission::ProcessMetadata));
    }

    #[test]
    fn restoring_empty_states_grants_nothing() {
        let manager = PermissionManager::from_states(&BTreeMap::new());

        for permission in [
            Permission::HardwareMetadata,
            Permission::ProcessMetadata,
            Permission::SoftwareInventory,
            Permission::FilesystemMetadata,
            Permission::NetworkMetadata,
        ] {
            assert_eq!(manager.get_state(permission), PermissionState::NotRequested);
            assert!(!manager.is_granted(permission));
        }
    }

    #[test]
    fn restored_permission_states_can_be_changed() {
        let mut saved = BTreeMap::new();
        saved.insert(Permission::HardwareMetadata, PermissionState::Granted);

        let mut manager = PermissionManager::from_states(&saved);
        manager.revoke(Permission::HardwareMetadata);

        assert_eq!(
            manager.get_state(Permission::HardwareMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn can_change_permission_decision() {
        let mut manager = PermissionManager::new();

        manager.grant(Permission::HardwareMetadata);
        manager.deny(Permission::HardwareMetadata);

        assert_eq!(
            manager.get_state(Permission::HardwareMetadata),
            PermissionState::Denied
        );
    }
}
