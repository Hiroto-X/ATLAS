use super::config::{ConfigManager, RuntimeConfig};
use super::error::RuntimeError;
use super::lifecycle::LifecycleManager;
use super::state::RuntimeStatus;
use super::permissions::{Permission, PermissionManager, PermissionState};

pub struct RuntimeHost {
    lifecycle: LifecycleManager,
    config: RuntimeConfig,
    permissions: PermissionManager,
}

impl RuntimeHost {
    pub fn new(config_manager: &ConfigManager) -> Result<Self, RuntimeError> {
        let config = config_manager.load().map_err(|error| {
            RuntimeError::ConfigurationFailed(error.to_string())
        })?;

        Ok(Self {
            lifecycle: LifecycleManager::new(),
            permissions: PermissionManager::new(),
            config,
        })
    }

    pub fn start(&mut self) -> Result<(), RuntimeError> {
        self.lifecycle.start()
    }

    pub fn pause(&mut self) -> Result<(), RuntimeError> {
        self.lifecycle.pause()
    }

    pub fn resume(&mut self) -> Result<(), RuntimeError> {
        self.lifecycle.resume()
    }

    pub fn begin_shutdown(&mut self) -> Result<(), RuntimeError> {
        self.lifecycle.begin_shutdown()
    }

    pub fn complete_shutdown(&mut self) -> Result<(), RuntimeError> {
        self.lifecycle.complete_shutdown()
    }

    pub fn status(&self) -> &RuntimeStatus {
        self.lifecycle.status()
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn permission_state(&self, permission: Permission) -> PermissionState {
        self.permissions.get_state(permission)
    }

    pub fn grant_permission(&mut self, permission: Permission) {
        self.permissions.grant(permission);
    }

    pub fn deny_permission(&mut self, permission: Permission) {
        self.permissions.deny(permission);
    }

    pub fn revoke_permission(&mut self, permission: Permission) {
        self.permissions.revoke(permission);
    }

    pub fn is_authorized(&self, permission: Permission) -> bool {
        self.permissions.is_granted(permission)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_host() -> (tempfile::TempDir, RuntimeHost) {
        let directory = tempdir().unwrap();

        let manager = ConfigManager::new_for_test (
            directory.path().join("runtime.json"),
        );

        let host = RuntimeHost::new(&manager).unwrap();

        (directory, host)
    }

    #[test]
    fn runtime_starts_successfully() {
        let (_directory, mut host) = create_host();

        assert_eq!(*host.status(), RuntimeStatus::Starting);

        host.start().unwrap();

        assert_eq!(*host.status(), RuntimeStatus::Running);
    }

    #[test]
    fn runtime_can_be_paused() {
        let (_directory, mut host) = create_host();

        host.start().unwrap();
        host.pause().unwrap();

        assert_eq!(*host.status(), RuntimeStatus::Paused);
    }

    #[test]
    fn runtime_can_shutdown_from_running() {
        let (_directory, mut host) = create_host();

        host.start().unwrap();
        host.begin_shutdown().unwrap();

        assert_eq!(*host.status(), RuntimeStatus::ShuttingDown);

        host.complete_shutdown().unwrap();

        assert_eq!(*host.status(), RuntimeStatus::Stopped);
    }

    #[test]
    fn runtime_rejects_invalid_start() {
        let (_directory, mut host) = create_host();

        host.start().unwrap();

        assert!(host.start().is_err());
    }

    #[test]
    fn runtime_rejects_pause_before_start() {
        let (_directory, mut host) = create_host();

        assert!(host.pause().is_err());
    }

    #[test]
    fn runtime_loads_configuration() {
        let (_directory, host) = create_host();

        assert_eq!(host.config().schema_version, 1);
        assert!(!host.config().background_enabled);
        assert!(!host.config().launch_at_login);
    }

    #[test]
    fn permissions_are_not_granted_by_default() {
        let config_manager =
            ConfigManager::new_for_test(std::env::temp_dir().join("atlas_test_default.json"));

        let host = RuntimeHost::new(&config_manager).unwrap();

        assert!(!host.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn runtime_host_can_grant_permission() {
        let config_manager =
            ConfigManager::new_for_test(std::env::temp_dir().join("atlas_test_grant.json"));

        let mut host = RuntimeHost::new(&config_manager).unwrap();

        host.grant_permission(Permission::HardwareMetadata);

        assert!(host.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::Granted
        );
    }

    #[test]
    fn runtime_host_can_deny_permission() {
        let config_manager =
            ConfigManager::new_for_test(std::env::temp_dir().join("atlas_test_deny.json"));

        let mut host = RuntimeHost::new(&config_manager).unwrap();

        host.deny_permission(Permission::ProcessMetadata);

        assert!(!host.is_authorized(Permission::ProcessMetadata));
        assert_eq!(
            host.permission_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );
    }

    #[test]
    fn runtime_host_can_revoke_permission() {
        let config_manager =
            ConfigManager::new_for_test(std::env::temp_dir().join("atlas_test_revoke.json"));

        let mut host = RuntimeHost::new(&config_manager).unwrap();

        host.grant_permission(Permission::NetworkMetadata);
        host.revoke_permission(Permission::NetworkMetadata);

        assert!(!host.is_authorized(Permission::NetworkMetadata));
        assert_eq!(
            host.permission_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn permissions_are_independent_inside_runtime_host() {
        let config_manager =
            ConfigManager::new_for_test(std::env::temp_dir().join("atlas_test_independent.json"));

        let mut host = RuntimeHost::new(&config_manager).unwrap();

        host.grant_permission(Permission::HardwareMetadata);
        host.deny_permission(Permission::NetworkMetadata);

        assert!(host.is_authorized(Permission::HardwareMetadata));
        assert!(!host.is_authorized(Permission::NetworkMetadata));
        assert!(!host.is_authorized(Permission::ProcessMetadata));
    }
}