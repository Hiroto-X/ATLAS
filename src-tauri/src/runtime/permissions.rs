use std::collections::HashMap;
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    HardwareMetadata,
    ProcessMetadata,
    SoftwareInventory,
    FilesystemMetadata,
    NetworkMetadata,
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
        self.permissions
            .insert(permission, PermissionState::Denied);
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