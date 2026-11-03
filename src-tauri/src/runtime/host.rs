use super::config::{ConfigDiagnostics, ConfigManager, RuntimeConfig};
use super::error::RuntimeError;
use super::lifecycle::LifecycleManager;
use super::permissions::{Permission, PermissionManager, PermissionState};
use super::state::RuntimeStatus;

pub struct RuntimeHost {
    lifecycle: LifecycleManager,
    config: RuntimeConfig,
    config_diagnostics: ConfigDiagnostics,
    config_manager: ConfigManager,
    permissions: PermissionManager,
}

impl RuntimeHost {
    pub fn new(config_manager: &ConfigManager) -> Result<Self, RuntimeError> {
        let report = config_manager
            .load_with_report()
            .map_err(|error| RuntimeError::ConfigurationFailed(error.to_string()))?;

        let permissions = PermissionManager::from_states(&report.config.permission_states);

        Ok(Self {
            lifecycle: LifecycleManager::new(),
            permissions,
            config: report.config,
            config_diagnostics: report.diagnostics,
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

    /// What happened when the configuration was loaded: warnings, and whether
    /// an unusable configuration was set aside and replaced by the defaults.
    pub fn config_diagnostics(&self) -> &ConfigDiagnostics {
        &self.config_diagnostics
    }

    pub fn permission_state(&self, permission: Permission) -> PermissionState {
        self.permissions.get_state(permission)
    }

    /// Grants a permission and saves the decision to `runtime.json`.
    ///
    /// The decision is saved first and only applied in memory once the save
    /// succeeds. If it cannot be saved, `ConfigurationFailed` is returned and
    /// the permission is left unchanged.
    pub fn grant_permission(&mut self, permission: Permission) -> Result<(), RuntimeError> {
        self.persist_permission_state(permission, PermissionState::Granted)
    }

    /// Denies a permission and saves the decision to `runtime.json`.
    ///
    /// The denial takes effect in memory immediately and is saved afterwards.
    /// If it cannot be saved, `PermissionNotPersisted` is returned and the
    /// permission stays denied in memory.
    pub fn deny_permission(&mut self, permission: Permission) -> Result<(), RuntimeError> {
        self.persist_permission_state(permission, PermissionState::Denied)
    }

    /// Resets a permission to `NotRequested` and saves the change to
    /// `runtime.json`.
    ///
    /// Authorization is removed in memory immediately and the change is saved
    /// afterwards. If it cannot be saved, `PermissionNotPersisted` is returned
    /// and the permission stays revoked in memory.
    pub fn revoke_permission(&mut self, permission: Permission) -> Result<(), RuntimeError> {
        self.persist_permission_state(permission, PermissionState::NotRequested)
    }

    pub fn is_authorized(&self, permission: Permission) -> bool {
        self.permissions.is_granted(permission)
    }

    /// Applies a permission decision and saves it to `runtime.json`.
    ///
    /// The order depends on the direction of the change:
    ///
    /// * `Granted` is saved first and only applied in memory once the save
    ///   succeeds, so a permission is never active without having been
    ///   persisted. A failed save returns `ConfigurationFailed` and changes
    ///   nothing.
    /// * `Denied` and `NotRequested` (revoke) remove authorization, so they
    ///   are applied in memory first and saved afterwards. A failed save
    ///   returns `PermissionNotPersisted` and the restriction stays in effect.
    ///
    /// The permission manager and `self.config` are always updated together,
    /// so they stay consistent. Every save writes the full configuration, so a
    /// restriction that could not be saved is written by the next successful
    /// save of any permission.
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

        if state == PermissionState::Granted {
            self.config_manager.save(&updated_config).map_err(|error| {
                RuntimeError::ConfigurationFailed(format!(
                    "failed to persist {permission:?} permission change: {error}"
                ))
            })?;

            self.permissions.grant(permission);
            self.config = updated_config;

            return Ok(());
        }

        // Denying or revoking removes authorization, so it takes effect before
        // the save is attempted and is kept even if the save fails.
        if state == PermissionState::Denied {
            self.permissions.deny(permission);
        } else {
            self.permissions.revoke(permission);
        }

        self.config = updated_config;

        self.config_manager.save(&self.config).map_err(|error| {
            RuntimeError::PermissionNotPersisted(format!(
                "{permission:?} was set to {state:?} in memory, but saving it failed: {error}"
            ))
        })
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

    fn restore_storage(manager: &ConfigManager) {
        fs::create_dir(manager.path().parent().unwrap()).unwrap();
    }

    const ALL_PERMISSIONS: [Permission; 5] = [
        Permission::HardwareMetadata,
        Permission::ProcessMetadata,
        Permission::SoftwareInventory,
        Permission::FilesystemMetadata,
        Permission::NetworkMetadata,
    ];

    /// The permission manager and the configuration mirror must never disagree.
    fn assert_memory_and_config_agree(host: &RuntimeHost) {
        for permission in ALL_PERMISSIONS {
            let mirrored = host
                .config()
                .permission_states
                .get(&permission)
                .copied()
                .unwrap_or(PermissionState::NotRequested);

            assert_eq!(
                host.permission_state(permission),
                mirrored,
                "{permission:?}"
            );
        }
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
    fn diagnostics_are_empty_for_a_valid_configuration() {
        let (_directory, manager) = create_manager();
        fs::write(
            manager.path(),
            r#"{"schema_version": 1, "permission_states": {"hardware_metadata": "granted"}}"#,
        )
        .unwrap();

        let host = RuntimeHost::new(&manager).unwrap();

        assert_eq!(*host.config_diagnostics(), ConfigDiagnostics::default());
        assert!(host.is_authorized(Permission::HardwareMetadata));
    }

    #[test]
    fn host_reports_recovery_and_starts_with_nothing_granted() {
        let (_directory, manager) = create_manager();
        let corrupt = r#"{"permission_states": {"hardware_metadata": "granted", "process_metadata": "maybe"}}"#;
        fs::write(manager.path(), corrupt).unwrap();

        let host = RuntimeHost::new(&manager).unwrap();
        let recovery = host
            .config_diagnostics()
            .recovery
            .as_ref()
            .expect("recovery expected");

        assert!(recovery.reason.contains("invalid state"));
        assert_eq!(
            fs::read_to_string(&recovery.quarantined_path).unwrap(),
            corrupt
        );
        for permission in ALL_PERMISSIONS {
            assert!(!host.is_authorized(permission), "{permission:?}");
        }
        assert_memory_and_config_agree(&host);
    }

    #[test]
    fn host_refuses_to_start_on_a_newer_configuration_and_leaves_it_alone() {
        let (_directory, manager) = create_manager();
        let newer =
            r#"{"schema_version": 2, "permission_states": {"hardware_metadata": "granted"}}"#;
        fs::write(manager.path(), newer).unwrap();

        let error = RuntimeHost::new(&manager).err().expect("start must fail");

        assert!(matches!(error, RuntimeError::ConfigurationFailed(_)));
        assert!(error.to_string().contains("schema version 2"));
        assert!(error.to_string().contains("not modified"));
        assert_eq!(fs::read_to_string(manager.path()).unwrap(), newer);
    }

    #[test]
    fn unknown_permissions_never_authorize_anything() {
        let (_directory, manager) = create_manager();
        fs::write(
            manager.path(),
            r#"{"permission_states": {"camera_access": "granted", "everything": "granted"}}"#,
        )
        .unwrap();

        let host = RuntimeHost::new(&manager).unwrap();

        assert_eq!(host.config_diagnostics().warnings.len(), 2);
        assert!(host.config_diagnostics().recovery.is_none());
        for permission in ALL_PERMISSIONS {
            assert!(!host.is_authorized(permission), "{permission:?}");
        }
    }

    #[test]
    fn unknown_entries_survive_permission_changes() {
        let (_directory, manager) = create_manager();
        fs::write(
            manager.path(),
            r#"{
              "schema_version": 1,
              "future_flag": true,
              "permission_states": {"camera_access": "granted"}
            }"#,
        )
        .unwrap();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.grant_permission(Permission::HardwareMetadata).unwrap();
        host.deny_permission(Permission::NetworkMetadata).unwrap();
        host.revoke_permission(Permission::HardwareMetadata)
            .unwrap();
        drop(host);

        let saved = manager.load().unwrap();

        assert_eq!(saved.unknown_fields["future_flag"], serde_json::json!(true));
        assert_eq!(
            saved.unknown_permissions["camera_access"],
            serde_json::json!("granted")
        );
        assert_eq!(
            saved.permission_states.get(&Permission::NetworkMetadata),
            Some(&PermissionState::Denied)
        );
    }

    #[test]
    fn recovered_host_persists_new_decisions_normally() {
        let (_directory, manager) = create_manager();
        fs::write(manager.path(), "{ corrupt").unwrap();

        let mut host = RuntimeHost::new(&manager).unwrap();
        host.grant_permission(Permission::HardwareMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert!(restored.config_diagnostics().recovery.is_none());
        assert!(restored.is_authorized(Permission::HardwareMetadata));
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
    fn deny_failure_keeps_denied_in_memory_and_reports_not_persisted() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::ProcessMetadata).unwrap();
        remove_storage(&manager);

        let error = host
            .deny_permission(Permission::ProcessMetadata)
            .unwrap_err();

        assert!(matches!(error, RuntimeError::PermissionNotPersisted(_)));
        assert!(error.to_string().contains("not persisted"));
        assert!(error.to_string().contains("ProcessMetadata"));
        assert!(!host.is_authorized(Permission::ProcessMetadata));
        assert_eq!(
            host.permission_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );
        assert_eq!(
            host.config()
                .permission_states
                .get(&Permission::ProcessMetadata),
            Some(&PermissionState::Denied)
        );
        assert_memory_and_config_agree(&host);
    }

    #[test]
    fn revoke_failure_keeps_revoked_state_in_memory_and_reports_not_persisted() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::NetworkMetadata).unwrap();
        remove_storage(&manager);

        let error = host
            .revoke_permission(Permission::NetworkMetadata)
            .unwrap_err();

        assert!(matches!(error, RuntimeError::PermissionNotPersisted(_)));
        assert!(error.to_string().contains("not persisted"));
        assert!(error.to_string().contains("NetworkMetadata"));
        assert!(!host.is_authorized(Permission::NetworkMetadata));
        assert_eq!(
            host.permission_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
        assert!(host
            .config()
            .permission_states
            .get(&Permission::NetworkMetadata)
            .is_none());
        assert_memory_and_config_agree(&host);
    }

    #[test]
    fn deny_failure_on_a_never_granted_permission_still_records_denied() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();
        remove_storage(&manager);

        let result = host.deny_permission(Permission::ProcessMetadata);

        assert!(matches!(
            result,
            Err(RuntimeError::PermissionNotPersisted(_))
        ));
        assert!(!host.is_authorized(Permission::ProcessMetadata));
        assert_eq!(
            host.permission_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );
        assert_memory_and_config_agree(&host);
    }

    #[test]
    fn unsaved_deny_is_persisted_by_a_later_successful_save() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::HardwareMetadata).unwrap();
        remove_storage(&manager);

        assert!(host.deny_permission(Permission::HardwareMetadata).is_err());

        restore_storage(&manager);
        host.grant_permission(Permission::NetworkMetadata).unwrap();

        let saved = manager.load().unwrap();

        assert_eq!(
            saved.permission_states.get(&Permission::HardwareMetadata),
            Some(&PermissionState::Denied)
        );
        assert_eq!(
            saved.permission_states.get(&Permission::NetworkMetadata),
            Some(&PermissionState::Granted)
        );
        assert_memory_and_config_agree(&host);
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert_eq!(
            restored.permission_state(Permission::HardwareMetadata),
            PermissionState::Denied
        );
        assert!(!restored.is_authorized(Permission::HardwareMetadata));
        assert!(restored.is_authorized(Permission::NetworkMetadata));
    }

    #[test]
    fn unsaved_revoke_is_persisted_by_a_later_successful_save() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::HardwareMetadata).unwrap();
        remove_storage(&manager);

        assert!(host
            .revoke_permission(Permission::HardwareMetadata)
            .is_err());

        restore_storage(&manager);
        host.deny_permission(Permission::NetworkMetadata).unwrap();

        let saved = manager.load().unwrap();

        assert!(saved
            .permission_states
            .get(&Permission::HardwareMetadata)
            .is_none());
        assert_eq!(
            saved.permission_states.get(&Permission::NetworkMetadata),
            Some(&PermissionState::Denied)
        );
        assert_memory_and_config_agree(&host);
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert!(!restored.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            restored.permission_state(Permission::HardwareMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn retrying_a_failed_deny_persists_it() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::ProcessMetadata).unwrap();
        remove_storage(&manager);

        assert!(host.deny_permission(Permission::ProcessMetadata).is_err());

        restore_storage(&manager);
        host.deny_permission(Permission::ProcessMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert_eq!(
            restored.permission_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );
    }

    #[test]
    fn failed_grant_does_not_discard_an_earlier_unsaved_restriction() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::HardwareMetadata).unwrap();
        remove_storage(&manager);

        assert!(matches!(
            host.deny_permission(Permission::HardwareMetadata),
            Err(RuntimeError::PermissionNotPersisted(_))
        ));
        assert!(matches!(
            host.grant_permission(Permission::NetworkMetadata),
            Err(RuntimeError::ConfigurationFailed(_))
        ));

        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::Denied
        );
        assert_eq!(
            host.permission_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
        assert_memory_and_config_agree(&host);

        restore_storage(&manager);
        host.grant_permission(Permission::NetworkMetadata).unwrap();
        drop(host);

        let restored = RuntimeHost::new(&manager).unwrap();

        assert_eq!(
            restored.permission_state(Permission::HardwareMetadata),
            PermissionState::Denied
        );
        assert!(restored.is_authorized(Permission::NetworkMetadata));
    }

    #[test]
    fn memory_and_config_stay_consistent_after_mixed_failed_changes() {
        let (_directory, manager, mut host) = create_host_with_removable_storage();

        host.grant_permission(Permission::HardwareMetadata).unwrap();
        host.grant_permission(Permission::NetworkMetadata).unwrap();
        remove_storage(&manager);

        assert!(host.deny_permission(Permission::HardwareMetadata).is_err());
        assert!(host.revoke_permission(Permission::NetworkMetadata).is_err());
        assert!(host.grant_permission(Permission::ProcessMetadata).is_err());

        assert_eq!(
            host.permission_state(Permission::HardwareMetadata),
            PermissionState::Denied
        );
        assert_eq!(
            host.permission_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
        assert_eq!(
            host.permission_state(Permission::ProcessMetadata),
            PermissionState::NotRequested
        );
        assert!(!host.is_authorized(Permission::HardwareMetadata));
        assert!(!host.is_authorized(Permission::NetworkMetadata));
        assert!(!host.is_authorized(Permission::ProcessMetadata));
        assert_memory_and_config_agree(&host);
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
