//! The application-facing side of the runtime.
//!
//! `RuntimeService` owns the one `RuntimeHost` of the application, behind a
//! mutex, together with the lock that keeps other ATLAS processes away from
//! the same configuration. It has no dependency on Tauri: the Tauri commands
//! in `lib.rs` only forward to it, so everything here is testable on its own.
//!
//! Everything returned to the user interface is made of plain data transfer
//! objects (`RuntimeStatusDto`, `PermissionDto`) and `IpcError`. Error
//! messages are fixed text. They never include file paths, operating system
//! error text or the contents of the configuration.

use std::sync::{Mutex, PoisonError};

use serde::Serialize;

use super::config::{
    instance_lock::{InstanceLock, InstanceLockError},
    ConfigManager,
};
use super::error::RuntimeError;
use super::host::RuntimeHost;
use super::permissions::{Permission, PermissionState};
use super::state::RuntimeStatus;

/// Name the per-user configuration directory is derived from.
pub const APP_CONFIG_NAME: &str = "ATLAS";

/// Why the runtime could not start. The application keeps running without a
/// runtime, and reports this through `RuntimeService::status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupFailure {
    /// Another ATLAS is already using this configuration.
    AlreadyRunning,
    /// The configuration could not be loaded.
    Configuration,
    /// Anything else that stopped the runtime from starting.
    Initialization,
}

impl StartupFailure {
    fn error(self) -> IpcError {
        match self {
            StartupFailure::AlreadyRunning => IpcError::new(
                "already_running",
                "ATLAS is already running. Close the other ATLAS window or process and try again.",
            ),
            StartupFailure::Configuration => IpcError::new(
                "configuration_unavailable",
                "ATLAS could not load its saved configuration. Make sure the file is valid and \
                 accessible, or update ATLAS if it was written by a newer version.",
            ),
            StartupFailure::Initialization => IpcError::new(
                "initialization_failed",
                "The ATLAS runtime could not start.",
            ),
        }
    }
}

/// An error that is safe to show in the user interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IpcError {
    /// Stable identifier the interface can act on.
    pub code: &'static str,
    /// Fixed, human-readable text.
    pub message: String,
}

impl IpcError {
    fn new(code: &'static str, message: &str) -> Self {
        Self {
            code,
            message: message.to_string(),
        }
    }

    fn internal() -> Self {
        Self::new(
            "internal_error",
            "The ATLAS runtime is in an unexpected state and cannot be used.",
        )
    }
}

impl From<&RuntimeError> for IpcError {
    /// Drops the details of the error on purpose: they can contain file paths.
    fn from(error: &RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidState(_) => IpcError::new(
                "invalid_state",
                "That operation is not allowed in the current runtime state.",
            ),
            RuntimeError::InitializationFailed(_) => IpcError::new(
                "initialization_failed",
                "The ATLAS runtime could not start.",
            ),
            RuntimeError::ShutdownFailed(_) => IpcError::new(
                "shutdown_failed",
                "The ATLAS runtime could not shut down cleanly.",
            ),
            RuntimeError::ConfigurationFailed(_) => IpcError::new(
                "configuration_failed",
                "The ATLAS configuration could not be read or written. The change was not applied.",
            ),
            RuntimeError::PermissionNotPersisted(_) => IpcError::new(
                "permission_not_persisted",
                "The permission change is in effect for this session, but it could not be \
                 saved and may be lost when ATLAS restarts.",
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeStateDto {
    Starting,
    Running,
    Paused,
    ShuttingDown,
    Stopped,
    /// There is no usable runtime. See `RuntimeStatusDto::error`.
    Unavailable,
}

impl From<&RuntimeStatus> for RuntimeStateDto {
    fn from(status: &RuntimeStatus) -> Self {
        match status {
            RuntimeStatus::Starting => RuntimeStateDto::Starting,
            RuntimeStatus::Running => RuntimeStateDto::Running,
            RuntimeStatus::Paused => RuntimeStateDto::Paused,
            RuntimeStatus::ShuttingDown => RuntimeStateDto::ShuttingDown,
            RuntimeStatus::Stopped => RuntimeStateDto::Stopped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatusDto {
    pub state: RuntimeStateDto,
    /// True only while the runtime is running.
    pub ready: bool,
    /// Why there is no usable runtime, if that is the case.
    pub error: Option<IpcError>,
    /// True if the saved configuration was unusable and was replaced by the
    /// defaults, which grant nothing.
    pub config_recovered: bool,
    /// Fixed explanations of anything the user should know about.
    pub notices: Vec<String>,
}

impl RuntimeStatusDto {
    fn unavailable(error: IpcError) -> Self {
        Self {
            state: RuntimeStateDto::Unavailable,
            ready: false,
            error: Some(error),
            config_recovered: false,
            notices: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDto {
    pub permission: Permission,
    pub state: PermissionState,
    pub authorized: bool,
}

/// What a running service holds. Field order matters: the host is dropped
/// before the lock is released.
struct Running {
    host: Mutex<RuntimeHost>,
    _lock: InstanceLock,
}

enum ServiceState {
    Running(Running),
    Unavailable(StartupFailure),
}

pub struct RuntimeService {
    state: ServiceState,
}

impl RuntimeService {
    /// Starts the runtime for the configuration in the user's Local
    /// application data directory.
    ///
    /// The single-instance lock is taken before `ConfigManager::new`, because
    /// that may migrate the old configuration and must not run twice at once.
    ///
    /// This never fails: if the runtime cannot start, the service reports why
    /// through `status`.
    pub fn start_default() -> Self {
        let directory = match ConfigManager::local_config_dir(APP_CONFIG_NAME) {
            Ok(directory) => directory,
            Err(error) => return Self::unavailable(StartupFailure::Configuration, &error),
        };

        if let Err(error) = std::fs::create_dir_all(&directory) {
            return Self::unavailable(StartupFailure::Configuration, &error);
        }

        let lock = match InstanceLock::acquire(
            &directory.join(super::config::instance_lock::LOCK_FILE_NAME),
        ) {
            Ok(lock) => lock,
            Err(error) => return Self::lock_failed(error),
        };

        match ConfigManager::new(APP_CONFIG_NAME) {
            Ok(manager) => Self::start_with_lock(&manager, lock),
            Err(error) => Self::unavailable(StartupFailure::Configuration, &error),
        }
    }

    /// Starts the runtime for `config_manager`, taking the single-instance
    /// lock next to its configuration first.
    pub fn start(config_manager: &ConfigManager) -> Self {
        match InstanceLock::acquire(&config_manager.lock_path()) {
            Ok(lock) => Self::start_with_lock(config_manager, lock),
            Err(error) => Self::lock_failed(error),
        }
    }

    fn start_with_lock(config_manager: &ConfigManager, lock: InstanceLock) -> Self {
        let mut host = match RuntimeHost::new(config_manager) {
            Ok(host) => host,
            Err(error) => return Self::unavailable(StartupFailure::Configuration, &error),
        };

        if let Err(error) = host.start() {
            return Self::unavailable(StartupFailure::Initialization, &error);
        }

        Self {
            state: ServiceState::Running(Running {
                host: Mutex::new(host),
                _lock: lock,
            }),
        }
    }

    fn lock_failed(error: InstanceLockError) -> Self {
        match error {
            InstanceLockError::AlreadyRunning => {
                Self::unavailable(StartupFailure::AlreadyRunning, &error)
            }
            InstanceLockError::Io(_) => Self::unavailable(StartupFailure::Initialization, &error),
        }
    }

    /// Records a startup failure. The detail goes to standard error for
    /// developers only; it is never part of what the interface receives.
    fn unavailable(failure: StartupFailure, detail: &dyn std::fmt::Display) -> Self {
        eprintln!("ATLAS runtime unavailable ({failure:?}): {detail}");

        Self {
            state: ServiceState::Unavailable(failure),
        }
    }

    /// The current state of the runtime.
    pub fn status(&self) -> RuntimeStatusDto {
        let running = match &self.state {
            ServiceState::Running(running) => running,
            ServiceState::Unavailable(failure) => {
                return RuntimeStatusDto::unavailable(failure.error());
            }
        };

        let Ok(host) = running.host.lock() else {
            return RuntimeStatusDto::unavailable(IpcError::internal());
        };

        let state = RuntimeStateDto::from(host.status());
        let diagnostics = host.config_diagnostics();
        let mut notices = Vec::new();

        if diagnostics.recovery.is_some() {
            notices.push(
                "The saved configuration could not be used and was set aside. Default \
                 settings were applied, and no permissions are granted."
                    .to_string(),
            );
        }

        if !diagnostics.warnings.is_empty() {
            notices.push(
                "Some unrecognised entries in the saved configuration were kept unchanged."
                    .to_string(),
            );
        }

        RuntimeStatusDto {
            state,
            ready: state == RuntimeStateDto::Running,
            error: None,
            config_recovered: diagnostics.recovery.is_some(),
            notices,
        }
    }

    /// Every permission and its current state.
    pub fn permissions(&self) -> Result<Vec<PermissionDto>, IpcError> {
        let host = self.lock_host()?;

        Ok(Permission::ALL
            .iter()
            .map(|permission| permission_dto(&host, *permission))
            .collect())
    }

    /// Grants a permission and saves the decision. If it cannot be saved,
    /// nothing is granted.
    pub fn grant_permission(&self, permission: Permission) -> Result<PermissionDto, IpcError> {
        self.change_permission(permission, RuntimeHost::grant_permission)
    }

    /// Denies a permission. The denial applies at once, even if it cannot be
    /// saved, in which case an error with the code `permission_not_persisted`
    /// is returned and `permissions` shows the denial.
    pub fn deny_permission(&self, permission: Permission) -> Result<PermissionDto, IpcError> {
        self.change_permission(permission, RuntimeHost::deny_permission)
    }

    /// Revokes a permission, with the same guarantees as `deny_permission`.
    pub fn revoke_permission(&self, permission: Permission) -> Result<PermissionDto, IpcError> {
        self.change_permission(permission, RuntimeHost::revoke_permission)
    }

    /// Shuts the runtime down. Meant for application exit, so it does not
    /// report failures.
    pub fn shutdown(&self) {
        let ServiceState::Running(running) = &self.state else {
            return;
        };

        // Exiting must not be blocked by a poisoned lock.
        let mut host = running.host.lock().unwrap_or_else(PoisonError::into_inner);

        let _ = host.shutdown();
    }

    fn change_permission(
        &self,
        permission: Permission,
        change: fn(&mut RuntimeHost, Permission) -> Result<(), RuntimeError>,
    ) -> Result<PermissionDto, IpcError> {
        let running = self.running()?;
        let mut host = running.host.lock().map_err(|_| IpcError::internal())?;

        change(&mut host, permission).map_err(|error| IpcError::from(&error))?;

        Ok(permission_dto(&host, permission))
    }

    fn lock_host(&self) -> Result<std::sync::MutexGuard<'_, RuntimeHost>, IpcError> {
        self.running()?
            .host
            .lock()
            .map_err(|_| IpcError::internal())
    }

    fn running(&self) -> Result<&Running, IpcError> {
        match &self.state {
            ServiceState::Running(running) => Ok(running),
            ServiceState::Unavailable(failure) => Err(failure.error()),
        }
    }
}

fn permission_dto(host: &RuntimeHost, permission: Permission) -> PermissionDto {
    PermissionDto {
        permission,
        state: host.permission_state(permission),
        authorized: host.is_authorized(permission),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, sync::Arc, thread};

    fn manager() -> (tempfile::TempDir, ConfigManager) {
        let directory = tempfile::tempdir().unwrap();
        let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));

        (directory, manager)
    }

    fn running(service: &RuntimeService) -> &Running {
        match &service.state {
            ServiceState::Running(running) => running,
            ServiceState::Unavailable(failure) => panic!("not running: {failure:?}"),
        }
    }

    fn unavailable_reason(service: &RuntimeService) -> StartupFailure {
        match &service.state {
            ServiceState::Unavailable(failure) => *failure,
            ServiceState::Running(_) => panic!("expected the service to be unavailable"),
        }
    }

    fn storage_service() -> (tempfile::TempDir, PathBuf, ConfigManager, RuntimeService) {
        let directory = tempfile::tempdir().unwrap();
        let storage = directory.path().join("storage");
        fs::create_dir(&storage).unwrap();
        let manager = ConfigManager::new_for_test(storage.join("runtime.json"));
        let service = RuntimeService::start(&manager);

        (directory, storage, manager, service)
    }

    // --- initialization -----------------------------------------------------

    #[test]
    fn runtime_initializes_and_runs() {
        let (_directory, manager) = manager();

        let service = RuntimeService::start(&manager);
        let status = service.status();

        assert_eq!(status.state, RuntimeStateDto::Running);
        assert!(status.ready);
        assert!(status.error.is_none());
        assert!(!status.config_recovered);
        assert!(status.notices.is_empty());
    }

    #[test]
    fn a_second_runtime_for_the_same_configuration_is_refused() {
        let (_directory, manager) = manager();
        let first = RuntimeService::start(&manager);

        let second = RuntimeService::start(&manager);

        assert_eq!(unavailable_reason(&second), StartupFailure::AlreadyRunning);
        assert_eq!(second.status().state, RuntimeStateDto::Unavailable);
        assert_eq!(second.status().error.unwrap().code, "already_running");
        // The first runtime is unaffected.
        assert_eq!(first.status().state, RuntimeStateDto::Running);
        assert!(first.grant_permission(Permission::HardwareMetadata).is_ok());
    }

    #[test]
    fn a_refused_runtime_cannot_change_permissions() {
        let (_directory, manager) = manager();
        let _first = RuntimeService::start(&manager);
        let second = RuntimeService::start(&manager);

        for result in [
            second.grant_permission(Permission::HardwareMetadata),
            second.deny_permission(Permission::HardwareMetadata),
            second.revoke_permission(Permission::HardwareMetadata),
        ] {
            assert_eq!(result.unwrap_err().code, "already_running");
        }
        assert_eq!(second.permissions().unwrap_err().code, "already_running");
    }

    #[test]
    fn the_lock_is_released_when_the_runtime_is_dropped() {
        let (_directory, manager) = manager();

        drop(RuntimeService::start(&manager));
        let again = RuntimeService::start(&manager);

        assert_eq!(again.status().state, RuntimeStateDto::Running);
    }

    #[test]
    fn runtimes_for_different_configurations_do_not_conflict() {
        let (_first_directory, first_manager) = manager();
        let (_second_directory, second_manager) = manager();

        let first = RuntimeService::start(&first_manager);
        let second = RuntimeService::start(&second_manager);

        assert!(first.status().ready);
        assert!(second.status().ready);
    }

    #[test]
    fn a_newer_configuration_leaves_the_runtime_unavailable_and_untouched() {
        let (directory, manager) = manager();
        let newer = r#"{"schema_version": 2}"#;
        fs::write(manager.path(), newer).unwrap();

        let service = RuntimeService::start(&manager);
        let status = service.status();
        let error = status.error.unwrap();

        assert_eq!(status.state, RuntimeStateDto::Unavailable);
        assert!(!status.ready);
        assert_eq!(error.code, "configuration_unavailable");
        assert!(!error
            .message
            .contains(&directory.path().display().to_string()));
        assert_eq!(fs::read_to_string(manager.path()).unwrap(), newer);
    }

    #[test]
    fn a_failed_start_does_not_keep_the_lock() {
        let (_directory, manager) = manager();
        fs::write(manager.path(), r#"{"schema_version": 2}"#).unwrap();

        let failed = RuntimeService::start(&manager);
        assert_eq!(unavailable_reason(&failed), StartupFailure::Configuration);

        // Fix the file. A later start must not be blocked by the failed one.
        fs::write(manager.path(), "{}").unwrap();
        let fixed = RuntimeService::start(&manager);

        assert_eq!(fixed.status().state, RuntimeStateDto::Running);
    }

    #[test]
    fn a_damaged_configuration_is_recovered_and_reported() {
        let (directory, manager) = manager();
        fs::write(
            manager.path(),
            r#"{"permission_states": {"hardware_metadata": "granted", "network_metadata": "x"}}"#,
        )
        .unwrap();

        let service = RuntimeService::start(&manager);
        let status = service.status();

        assert_eq!(status.state, RuntimeStateDto::Running);
        assert!(status.config_recovered);
        assert_eq!(status.notices.len(), 1);
        assert!(status.notices[0].contains("no permissions are granted"));
        assert!(service
            .permissions()
            .unwrap()
            .iter()
            .all(|dto| !dto.authorized && dto.state == PermissionState::NotRequested));

        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains(&directory.path().display().to_string()));
        assert!(!json.contains("corrupt"));
    }

    #[test]
    fn unrecognised_entries_are_reported_as_a_notice() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            r#"{"permission_states": {"camera_access": "granted"}}"#,
        )
        .unwrap();

        let status = RuntimeService::start(&manager).status();

        assert!(!status.config_recovered);
        assert_eq!(status.notices.len(), 1);
        assert!(status.notices[0].contains("unrecognised"));
        assert!(!status.notices[0].contains("camera_access"));
    }

    // --- permission commands ------------------------------------------------

    #[test]
    fn every_permission_starts_not_requested_and_unauthorized() {
        let (_directory, manager) = manager();
        let service = RuntimeService::start(&manager);

        let permissions = service.permissions().unwrap();

        assert_eq!(permissions.len(), Permission::ALL.len());
        for (dto, expected) in permissions.iter().zip(Permission::ALL) {
            assert_eq!(dto.permission, expected);
            assert_eq!(dto.state, PermissionState::NotRequested);
            assert!(!dto.authorized);
        }
    }

    #[test]
    fn grant_deny_and_revoke_return_the_new_state() {
        let (_directory, manager) = manager();
        let service = RuntimeService::start(&manager);

        let granted = service
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();
        assert_eq!(granted.state, PermissionState::Granted);
        assert!(granted.authorized);

        let denied = service
            .deny_permission(Permission::HardwareMetadata)
            .unwrap();
        assert_eq!(denied.state, PermissionState::Denied);
        assert!(!denied.authorized);

        let revoked = service
            .revoke_permission(Permission::HardwareMetadata)
            .unwrap();
        assert_eq!(revoked.state, PermissionState::NotRequested);
        assert!(!revoked.authorized);
    }

    #[test]
    fn permission_decisions_survive_a_restart() {
        let (_directory, manager) = manager();

        let first = RuntimeService::start(&manager);
        first
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();
        first.deny_permission(Permission::NetworkMetadata).unwrap();
        drop(first);

        let second = RuntimeService::start(&manager);
        let permissions = second.permissions().unwrap();
        let find = |permission| {
            permissions
                .iter()
                .find(|d| d.permission == permission)
                .unwrap()
        };

        assert!(find(Permission::HardwareMetadata).authorized);
        assert_eq!(
            find(Permission::NetworkMetadata).state,
            PermissionState::Denied
        );
        assert_eq!(
            find(Permission::ProcessMetadata).state,
            PermissionState::NotRequested
        );
    }

    #[test]
    fn a_grant_that_cannot_be_saved_is_refused() {
        let (_directory, storage, _manager, service) = storage_service();
        fs::remove_dir_all(&storage).unwrap();

        let error = service
            .grant_permission(Permission::HardwareMetadata)
            .unwrap_err();

        assert_eq!(error.code, "configuration_failed");
        assert!(!service
            .permissions()
            .unwrap()
            .iter()
            .any(|dto| dto.authorized));
    }

    #[test]
    fn a_denial_that_cannot_be_saved_still_applies() {
        let (_directory, storage, _manager, service) = storage_service();
        service
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();
        fs::remove_dir_all(&storage).unwrap();

        let error = service
            .deny_permission(Permission::HardwareMetadata)
            .unwrap_err();
        let permissions = service.permissions().unwrap();
        let hardware = permissions
            .iter()
            .find(|dto| dto.permission == Permission::HardwareMetadata)
            .unwrap();

        assert_eq!(error.code, "permission_not_persisted");
        assert_eq!(hardware.state, PermissionState::Denied);
        assert!(!hardware.authorized);
    }

    #[test]
    fn a_revocation_that_cannot_be_saved_still_applies() {
        let (_directory, storage, _manager, service) = storage_service();
        service
            .grant_permission(Permission::NetworkMetadata)
            .unwrap();
        fs::remove_dir_all(&storage).unwrap();

        let error = service
            .revoke_permission(Permission::NetworkMetadata)
            .unwrap_err();

        assert_eq!(error.code, "permission_not_persisted");
        assert!(!service
            .permissions()
            .unwrap()
            .iter()
            .any(|dto| dto.authorized));
    }

    #[test]
    fn concurrent_changes_are_all_applied_and_saved() {
        let (_directory, manager) = manager();
        let service = Arc::new(RuntimeService::start(&manager));

        let handles: Vec<_> = Permission::ALL
            .into_iter()
            .map(|permission| {
                let service = Arc::clone(&service);
                thread::spawn(move || {
                    for _ in 0..10 {
                        service.deny_permission(permission).unwrap();
                        service.grant_permission(permission).unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert!(service
            .permissions()
            .unwrap()
            .iter()
            .all(|dto| dto.authorized));

        let saved = manager.load().unwrap();
        assert_eq!(saved.permission_states.len(), Permission::ALL.len());
        assert!(saved
            .permission_states
            .values()
            .all(|state| *state == PermissionState::Granted));
    }

    // --- shutdown and failure of the runtime itself --------------------------

    #[test]
    fn shutdown_stops_the_runtime() {
        let (_directory, manager) = manager();
        let service = RuntimeService::start(&manager);

        service.shutdown();

        assert_eq!(service.status().state, RuntimeStateDto::Stopped);
        assert!(!service.status().ready);
    }

    #[test]
    fn shutting_down_twice_or_an_unavailable_runtime_does_not_panic() {
        let (_directory, manager) = manager();
        let service = RuntimeService::start(&manager);
        service.shutdown();
        service.shutdown();

        let refused = RuntimeService::start(&manager);
        refused.shutdown();

        assert_eq!(refused.status().state, RuntimeStateDto::Unavailable);
    }

    #[test]
    fn a_poisoned_runtime_fails_closed() {
        let (_directory, manager) = manager();
        let service = Arc::new(RuntimeService::start(&manager));
        service
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();

        let poisoner = Arc::clone(&service);
        let _ = thread::spawn(move || {
            let _guard = running(&poisoner).host.lock().unwrap();
            panic!("poison the lock on purpose");
        })
        .join();

        let status = service.status();
        assert_eq!(status.state, RuntimeStateDto::Unavailable);
        assert!(!status.ready);
        assert_eq!(status.error.unwrap().code, "internal_error");
        assert_eq!(service.permissions().unwrap_err().code, "internal_error");
        assert_eq!(
            service
                .grant_permission(Permission::NetworkMetadata)
                .unwrap_err()
                .code,
            "internal_error"
        );
        // Exiting still works.
        service.shutdown();
    }

    // --- error mapping and data shapes --------------------------------------

    #[test]
    fn every_runtime_error_maps_to_fixed_text_without_its_details() {
        let secret = r"C:\Users\someone\AppData\Local\atlas\runtime.json";
        let errors = [
            RuntimeError::InvalidState(secret.into()),
            RuntimeError::InitializationFailed(secret.into()),
            RuntimeError::ShutdownFailed(secret.into()),
            RuntimeError::ConfigurationFailed(secret.into()),
            RuntimeError::PermissionNotPersisted(secret.into()),
        ];
        let mut codes = Vec::new();

        for error in &errors {
            let mapped = IpcError::from(error);

            assert!(!mapped.message.contains(secret), "{error:?}");
            assert!(!mapped.message.contains("AppData"), "{error:?}");
            assert!(!mapped.message.is_empty());
            codes.push(mapped.code);
        }

        let mut unique = codes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), codes.len(), "each error has its own code");
    }

    #[test]
    fn startup_failures_have_distinct_safe_errors() {
        let failures = [
            StartupFailure::AlreadyRunning,
            StartupFailure::Configuration,
            StartupFailure::Initialization,
        ];
        let mut codes: Vec<&str> = failures
            .iter()
            .map(|failure| failure.error().code)
            .collect();

        codes.sort_unstable();
        codes.dedup();

        assert_eq!(codes.len(), failures.len());
        for failure in failures {
            let message = failure.error().message;

            assert!(!message.contains('\\'));
            assert!(!message.contains(".json"));
        }
    }

    #[test]
    fn status_serializes_with_camel_case_names() {
        let (_directory, manager) = manager();

        let value = serde_json::to_value(RuntimeService::start(&manager).status()).unwrap();

        assert_eq!(value["state"], "running");
        assert_eq!(value["ready"], true);
        assert_eq!(value["error"], serde_json::Value::Null);
        assert_eq!(value["configRecovered"], false);
        assert_eq!(value["notices"], serde_json::json!([]));
    }

    #[test]
    fn unavailable_status_serializes_its_error() {
        let (_directory, manager) = manager();
        let _first = RuntimeService::start(&manager);

        let value = serde_json::to_value(RuntimeService::start(&manager).status()).unwrap();

        assert_eq!(value["state"], "unavailable");
        assert_eq!(value["ready"], false);
        assert_eq!(value["error"]["code"], "already_running");
        assert!(value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already running"));
    }

    #[test]
    fn every_state_serializes_to_its_expected_name() {
        for (state, name) in [
            (RuntimeStateDto::Starting, "starting"),
            (RuntimeStateDto::Running, "running"),
            (RuntimeStateDto::Paused, "paused"),
            (RuntimeStateDto::ShuttingDown, "shuttingDown"),
            (RuntimeStateDto::Stopped, "stopped"),
            (RuntimeStateDto::Unavailable, "unavailable"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), name);
        }
    }

    #[test]
    fn permission_dto_uses_the_names_the_interface_sends_back() {
        let dto = PermissionDto {
            permission: Permission::HardwareMetadata,
            state: PermissionState::NotRequested,
            authorized: false,
        };

        assert_eq!(
            serde_json::to_value(dto).unwrap(),
            serde_json::json!({
                "permission": "hardware_metadata",
                "state": "not_requested",
                "authorized": false
            })
        );

        // The same name is accepted as a command argument.
        let parsed: Permission = serde_json::from_str("\"hardware_metadata\"").unwrap();
        assert_eq!(parsed, Permission::HardwareMetadata);
        assert!(serde_json::from_str::<Permission>("\"camera\"").is_err());
    }

    #[test]
    fn ipc_error_serializes_as_code_and_message() {
        let value = serde_json::to_value(IpcError::new("some_code", "Some text.")).unwrap();

        assert_eq!(
            value,
            serde_json::json!({"code": "some_code", "message": "Some text."})
        );
    }

    // --- lifecycle (2.6.6) ----------------------------------------------------

    #[test]
    fn permission_commands_are_rejected_after_shutdown() {
        let (_directory, manager) = manager();
        let service = RuntimeService::start(&manager);
        service
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();

        service.shutdown();

        for result in [
            service.grant_permission(Permission::NetworkMetadata),
            service.deny_permission(Permission::HardwareMetadata),
            service.revoke_permission(Permission::HardwareMetadata),
        ] {
            assert_eq!(result.unwrap_err().code, "invalid_state");
        }
    }

    #[test]
    fn nothing_is_reported_authorized_after_shutdown_but_decisions_remain_visible() {
        let (_directory, manager) = manager();
        let service = RuntimeService::start(&manager);
        service
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();

        service.shutdown();
        let permissions = service.permissions().unwrap();
        let hardware = permissions
            .iter()
            .find(|dto| dto.permission == Permission::HardwareMetadata)
            .unwrap();

        assert_eq!(hardware.state, PermissionState::Granted);
        assert!(!hardware.authorized);
        assert!(permissions.iter().all(|dto| !dto.authorized));
    }

    #[test]
    fn a_granted_permission_is_authorized_while_running() {
        let (_directory, manager) = manager();
        let service = RuntimeService::start(&manager);

        let granted = service
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();

        assert!(granted.authorized);
    }

    #[test]
    fn shutdown_keeps_decisions_for_the_next_start() {
        let (_directory, manager) = manager();
        let first = RuntimeService::start(&manager);
        first
            .grant_permission(Permission::HardwareMetadata)
            .unwrap();
        first.shutdown();
        drop(first);

        let second = RuntimeService::start(&manager);

        assert!(second
            .permissions()
            .unwrap()
            .iter()
            .any(|dto| dto.permission == Permission::HardwareMetadata && dto.authorized));
    }
}
