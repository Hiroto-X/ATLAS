use super::config::{ConfigManager, RuntimeConfig};
use super::error::RuntimeError;
use super::lifecycle::LifecycleManager;
use super::permissions::{Permission, PermissionManager, PermissionState};
use super::state::RuntimeStatus;

pub struct RuntimeHost {
    lifecycle: LifecycleManager,
    config: RuntimeConfig,
    config_manager: ConfigManager,
    permissions: PermissionManager,
}

impl RuntimeHost {
    pub fn new(config_manager: &ConfigManager) -> Result<Self, RuntimeError> {
        let config = config_manager
            .load()
            .map_err(|error| RuntimeError::ConfigurationFailed(error.to_string()))?;

        let permissions = PermissionManager::from_states(&config.permission_states);

        Ok(Self {
            lifecycle: LifecycleManager::new(),
            permissions,
            config,
            config_manager: config_manager.clone(),
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

    /// Grants a permission and saves the decision to `runtime.json`.
    ///
    /// If the decision cannot be saved, an error is returned and the
    /// permission is left unchanged.
    pub fn grant_permission(&mut self, permission: Permission) -> Result<(), RuntimeError> {
        self.persist_permission_state(permission, PermissionState::Granted)
    }

    /// Denies a permission and saves the decision to `runtime.json`.
    ///
    /// If the decision cannot be saved, an error is returned and the
    /// permission is left unchanged.
    pub fn deny_permission(&mut self, permission: Permission) -> Result<(), RuntimeError> {
        self.persist_permission_state(permission, PermissionState::Denied)
    }

    /// Resets a permission to `NotRequested` and saves the change to
    /// `runtime.json`.
    ///
    /// If the change cannot be saved, an error is returned and the previous
    /// decision is left in place.
    pub fn revoke_permission(&mut self, permission: Permission) -> Result<(), RuntimeError> {
        self.persist_permission_state(permission, PermissionState::NotRequested)
    }

    pub fn is_authorized(&self, permission: Permission) -> bool {
        self.permissions.is_granted(permission)
    }

    /// Saves a permission decision, then applies it in memory.
    ///
    /// The change is written to a copy of the configuration first. In-memory
    /// state is only updated once the save succeeds, so a permission can never
    /// be active in memory without having been persisted.
    fn persist_permission_state(
        &mut self,
        permission: Permission,
        state: PermissionState,
    ) -> Result<(), RuntimeError> {
        let mut updated_config = self.config.clone();

        match state {
            // NotRequested is the default, so it is stored as an absent entry.
            PermissionState::NotRequested => {
                updated_config.permission_states.remove(&permission);
            }
            PermissionState::Granted | PermissionState::Denied => {
                updated_config.permission_states.insert(permission, state);
            }
        }

        self.config_manager.save(&updated_config).map_err(|error| {
            RuntimeError::ConfigurationFailed(format!(
                "failed to persist {permission:?} permission change: {error}"
            ))
        })?;

        match state {
            PermissionState::Granted => self.permissions.grant(permission),
            PermissionState::Denied => self.permissions.deny(permission),
            PermissionState::NotRequested => self.permissions.revoke(permission),
        }

        self.config = updated_config;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn create_host() -> (tempfile::TempDir, RuntimeHost) {
        let directory = tempdir().unwrap();

        let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));

        let host = RuntimeHost::new(&manager).unwrap();

        (directory, host)
    }

    fn create_manager() -> (tempfile::TempDir, ConfigManager) {
        let directory = tempdir().unwrap();

        let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));

        (directory, manager)
    }

    /// Creates a host whose configuration lives in its own subdirectory, so
    /// tests can simulate a storage failure by removing that directory.
    fn create_host_with_removable_storage() -> (tempfile::TempDir, ConfigManager, RuntimeHost) {
        let directory = tempdir().unwrap();
        let storage = directory.path().join("storage");
        fs::create_dir(&storage).unwrap();

        let manager = ConfigManager::new_for_test(storage.join("runtime.json"));
        let host = RuntimeHost::new(&manager).unwrap();

        (directory, manager, host)
    }

    fn remove_storage(manager: &ConfigManager) {
        fs::remove_dir_all(manager.path().parent().unwrap()).unwrap();
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
        let (_directory, host) = create_host();

        assert!(!host.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn runtime_host_can_grant_permission() {
        let (_directory, mut host) = create_host();

        host.grant_permission(Permission::HardwareMetadata).unwrap();

        assert!(host.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::Granted
        );
    }

    #[test]
    fn runtime_host_can_deny_permission() {
        let (_directory, mut host) = create_host();

        host.deny_permission(Permission::ProcessMetadata).unwrap();

        assert!(!host.is_authorized(Permission::ProcessMetadata));
        assert_eq!(
            host.permission_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );
    }

    #[test]
    fn runtime_host_can_revoke_permission() {
        let (_directory, mut host) = create_host();

        host.grant_permission(Permission::NetworkMetadata).unwrap();
        host.revoke_permission(Permission::NetworkMetadata).unwrap();

        assert!(!host.is_authorized(Permission::NetworkMetadata));
        assert_eq!(
            host.permission_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn permissions_are_independent_inside_runtime_host() {
        let (_directory, mut host) = create_host();

        host.grant_permission(Permission::HardwareMetadata).unwrap();
        host.deny_permission(Permission::NetworkMetadata).unwrap();

        assert!(host.is_authorized(Permission::HardwareMetadata));
        assert!(!host.is_authorized(Permission::NetworkMetadata));
        assert!(!host.is_authorized(Permission::ProcessMetadata));
    }

    #[test]
    fn granted_permission_is_restored_by_a_new_host() {
        let (_directory, manager) = create_manager();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.grant_permission(Permission::HardwareMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert!(restored.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            restored.permission_state(Permission::HardwareMetadata),
            PermissionState::Granted
        );
    }

    #[test]
    fn denied_permission_is_restored_by_a_new_host() {
        let (_directory, manager) = create_manager();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.deny_permission(Permission::ProcessMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert!(!restored.is_authorized(Permission::ProcessMetadata));
        assert_eq!(
            restored.permission_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );
    }

    #[test]
    fn revoked_permission_is_not_restored_by_a_new_host() {
        let (_directory, manager) = create_manager();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.grant_permission(Permission::NetworkMetadata).unwrap();
        host.revoke_permission(Permission::NetworkMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert!(!restored.is_authorized(Permission::NetworkMetadata));
        assert_eq!(
            restored.permission_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
        assert!(manager
            .load()
            .unwrap()
            .permission_states
            .get(&Permission::NetworkMetadata)
            .is_none());
    }

    #[test]
    fn permission_changes_are_written_to_runtime_json() {
        let (_directory, manager) = create_manager();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.grant_permission(Permission::HardwareMetadata).unwrap();
        host.deny_permission(Permission::NetworkMetadata).unwrap();

        let saved = manager.load().unwrap();

        assert_eq!(
            saved.permission_states.get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
        assert_eq!(
            saved.permission_states.get(&Permission::NetworkMetadata),
            Some(&PermissionState::Denied)
        );
        assert_eq!(host.config().permission_states, saved.permission_states);
    }

    #[test]
    fn permission_decisions_persist_independently() {
        let (_directory, manager) = create_manager();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.grant_permission(Permission::HardwareMetadata).unwrap();
        host.deny_permission(Permission::NetworkMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert_eq!(
            restored.permission_state(Permission::HardwareMetadata),
            PermissionState::Granted
        );
        assert_eq!(
            restored.permission_state(Permission::NetworkMetadata),
            PermissionState::Denied
        );
        assert_eq!(
            restored.permission_state(Permission::ProcessMetadata),
            PermissionState::NotRequested
        );
        assert_eq!(
            restored.permission_state(Permission::SoftwareInventory),
            PermissionState::NotRequested
        );
        assert_eq!(
            restored.permission_state(Permission::FilesystemMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn latest_permission_decision_is_the_one_persisted() {
        let (_directory, manager) = create_manager();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.grant_permission(Permission::HardwareMetadata).unwrap();
        host.deny_permission(Permission::HardwareMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert_eq!(
            restored.permission_state(Permission::HardwareMetadata),
            PermissionState::Denied
        );
        assert!(!restored.is_authorized(Permission::HardwareMetadata));
    }

    #[test]
    fn legacy_configuration_without_permissions_still_loads() {
        let (_directory, manager) = create_manager();

        fs::write(
            manager.path(),
            r#"{"schema_version":1,"background_enabled":true,"launch_at_login":false}"#,
        )
        .unwrap();

        let mut host = RuntimeHost::new(&manager).unwrap();

        assert!(host.config().background_enabled);
        assert!(!host.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::NotRequested
        );

        host.grant_permission(Permission::HardwareMetadata).unwrap();

        let saved = manager.load().unwrap();

        assert!(saved.background_enabled);
        assert!(!saved.launch_at_login);
        assert_eq!(
            saved.permission_states.get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
    }

    #[test]
    fn grant_fails_explicitly_when_configuration_cannot_be_saved() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();
        remove_storage(&manager);

        let error = host
            .grant_permission(Permission::HardwareMetadata)
            .unwrap_err();

        assert!(matches!(error, RuntimeError::ConfigurationFailed(_)));
        assert!(error.to_string().contains("HardwareMetadata"));
        assert!(!host.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::NotRequested
        );
        assert!(host.config().permission_states.is_empty());
    }

    #[test]
    fn deny_failure_leaves_previous_decision_unchanged() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::ProcessMetadata).unwrap();
        remove_storage(&manager);

        let result = host.deny_permission(Permission::ProcessMetadata);

        assert!(matches!(result, Err(RuntimeError::ConfigurationFailed(_))));
        assert_eq!(
            host.permission_state(Permission::ProcessMetadata),
            PermissionState::Granted
        );
        assert_eq!(
            host.config()
                .permission_states
                .get(&Permission::ProcessMetadata),
            Some(&PermissionState::Granted)
        );
    }

    #[test]
    fn revoke_failure_leaves_permission_granted() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::NetworkMetadata).unwrap();
        remove_storage(&manager);

        let result = host.revoke_permission(Permission::NetworkMetadata);

        assert!(matches!(result, Err(RuntimeError::ConfigurationFailed(_))));
        assert!(host.is_authorized(Permission::NetworkMetadata));
        assert_eq!(
            host.permission_state(Permission::NetworkMetadata),
            PermissionState::Granted
        );
    }

    #[test]
    fn permission_change_succeeds_once_storage_is_available_again() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();
        remove_storage(&manager);

        assert!(host.grant_permission(Permission::HardwareMetadata).is_err());
        assert!(!host.is_authorized(Permission::HardwareMetadata));

        fs::create_dir(manager.path().parent().unwrap()).unwrap();

        host.grant_permission(Permission::HardwareMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert!(restored.is_authorized(Permission::HardwareMetadata));
    }
}
