use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::atomic::atomic_write;
use super::instance_lock::LOCK_FILE_NAME;
use super::migration::migrate_legacy_config;
use super::recovery::{quarantine, ConfigDiagnostics, ConfigRecovery, LoadReport};
use super::schema::RuntimeConfig;
use super::validation::{parse_config_bytes, ConfigParseError};

const CONFIG_FILE_NAME: &str = "runtime.json";

#[derive(Clone)]
pub struct ConfigManager {
    config_path: PathBuf,
}

impl ConfigManager {
    /// Creates a manager for the per-user configuration in the Local
    /// application data directory (`%LOCALAPPDATA%` on Windows).
    ///
    /// Configuration written by earlier builds to the Roaming directory is
    /// copied over once, if there is no Local configuration yet. See
    /// `migrate_legacy_config` for the exact rules. If that migration fails,
    /// an error is returned and the old configuration is left in place.
    pub fn new(app_name: &str) -> io::Result<Self> {
        let base_dir =
            directories::ProjectDirs::from("com", "atlas", app_name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "Unable to locate application configuration directory",
                )
            })?;

        // The directory layout below the base folder is the same for both; only
        // the base folder differs on Windows. On other platforms they are
        // identical and the migration does nothing.
        Self::from_directories(base_dir.config_local_dir(), base_dir.config_dir())
    }

    /// Creates a manager that stores `runtime.json` in `local_dir`, first
    /// migrating `legacy_dir/runtime.json` if applicable.
    pub(crate) fn from_directories(local_dir: &Path, legacy_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(local_dir)?;

        let config_path = local_dir.join(CONFIG_FILE_NAME);

        migrate_legacy_config(&legacy_dir.join(CONFIG_FILE_NAME), &config_path)?;

        Ok(Self { config_path })
    }

    /// Loads the configuration. See `load_with_report` for the rules; this
    /// discards the diagnostics.
    pub fn load(&self) -> io::Result<RuntimeConfig> {
        self.load_with_report().map(|report| report.config)
    }

    /// Loads the configuration and says what happened while doing so.
    ///
    /// * No file: the default configuration is written and returned.
    /// * Valid file: returned as read. Unrecognised entries are kept and
    ///   listed in the warnings.
    /// * File written by a newer ATLAS (`UnsupportedSchemaVersion`): an error
    ///   is returned and the file is not touched.
    /// * File that cannot be interpreted safely: it is moved aside
    ///   (quarantined) and the default configuration, which grants nothing, is
    ///   written in its place. `diagnostics.recovery` says where the original
    ///   went. If it cannot be moved aside, an error is returned and nothing
    ///   is replaced.
    /// * File that cannot be read at all: an error is returned.
    pub fn load_with_report(&self) -> io::Result<LoadReport> {
        self.load_with_report_using(quarantine)
    }

    /// `load_with_report` with the quarantine step supplied by the caller, so
    /// that its failure can be tested.
    fn load_with_report_using(
        &self,
        quarantine_file: impl Fn(&Path) -> io::Result<PathBuf>,
    ) -> io::Result<LoadReport> {
        let bytes = match fs::read(&self.config_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return self.create_default(ConfigDiagnostics::default());
            }
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "could not read configuration {}: {error}",
                        self.config_path.display()
                    ),
                ));
            }
        };

        match parse_config_bytes(&bytes) {
            Ok(parsed) => Ok(LoadReport {
                config: parsed.config,
                diagnostics: ConfigDiagnostics {
                    warnings: parsed.warnings,
                    recovery: None,
                },
            }),
            Err(error @ ConfigParseError::UnsupportedSchemaVersion { .. }) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "configuration {} cannot be used: {error}. The file was not modified.",
                    self.config_path.display()
                ),
            )),
            Err(ConfigParseError::Malformed(reason)) => self.recover(reason, quarantine_file),
        }
    }

    /// Sets the unusable configuration aside, then writes the default one.
    /// The default is only written after the original is safe.
    fn recover(
        &self,
        reason: String,
        quarantine_file: impl Fn(&Path) -> io::Result<PathBuf>,
    ) -> io::Result<LoadReport> {
        let quarantined_path = quarantine_file(&self.config_path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "configuration {} cannot be used ({reason}) and could not be set aside: \
                     {error}. The file was left in place.",
                    self.config_path.display()
                ),
            )
        })?;

        self.create_default(ConfigDiagnostics {
            warnings: Vec::new(),
            recovery: Some(ConfigRecovery {
                reason: reason.clone(),
                quarantined_path: quarantined_path.clone(),
            }),
        })
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "configuration {} cannot be used ({reason}). The original was kept as {}, \
                     but the default configuration could not be saved: {error}",
                    self.config_path.display(),
                    quarantined_path.display()
                ),
            )
        })
    }

    /// Writes the default configuration and returns it.
    fn create_default(&self, diagnostics: ConfigDiagnostics) -> io::Result<LoadReport> {
        let config = RuntimeConfig::default();

        self.save(&config)?;

        Ok(LoadReport {
            config,
            diagnostics,
        })
    }

    pub fn save(&self, config: &RuntimeConfig) -> io::Result<()> {
        let contents = serde_json::to_string_pretty(config)?;

        atomic_write(&self.config_path, contents.as_bytes())
    }

    pub fn path(&self) -> &Path {
        &self.config_path
    }

    /// The per-user directory `new` stores the configuration in. It is not
    /// created here. The runtime locks a file in it before calling `new`,
    /// because `new` may migrate the old configuration.
    pub fn local_config_dir(app_name: &str) -> io::Result<PathBuf> {
        let base_dir =
            directories::ProjectDirs::from("com", "atlas", app_name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "Unable to locate application configuration directory",
                )
            })?;

        Ok(base_dir.config_local_dir().to_path_buf())
    }

    /// Where the single-instance lock for this configuration lives.
    pub fn lock_path(&self) -> PathBuf {
        self.config_path.with_file_name(LOCK_FILE_NAME)
    }

    /// Reads the configuration as it is on disk right now, without creating,
    /// replacing or quarantining anything. `None` means there is no file.
    /// A file that cannot be read or interpreted is an error.
    pub fn read_current(&self) -> io::Result<Option<RuntimeConfig>> {
        let bytes = match fs::read(&self.config_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };

        match parse_config_bytes(&bytes) {
            Ok(parsed) => Ok(Some(parsed.config)),
            Err(error @ ConfigParseError::UnsupportedSchemaVersion { .. }) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                error.to_string(),
            )),
            Err(error) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(config_path: PathBuf) -> Self {
        Self { config_path }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::config::RuntimeConfig;
    use crate::runtime::permissions::{Permission, PermissionState};
    use std::collections::BTreeMap;

    fn create_test_manager() -> (tempfile::TempDir, ConfigManager) {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("runtime.json");

        let manager = ConfigManager { config_path };

        (directory, manager)
    }

    const ALL_PERMISSIONS: [Permission; 5] = [
        Permission::HardwareMetadata,
        Permission::ProcessMetadata,
        Permission::SoftwareInventory,
        Permission::FilesystemMetadata,
        Permission::NetworkMetadata,
    ];

    /// Quarantined copies (`runtime.json.corrupt-*`) next to the configuration.
    fn quarantined_files(manager: &ConfigManager) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(manager.path().parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("runtime.json.corrupt-")
            })
            .collect();

        found.sort();
        found
    }

    #[test]
    fn creates_default_configuration_when_file_is_missing() {
        let (_directory, manager) = create_test_manager();

        let config = manager.load().unwrap();

        assert_eq!(config.schema_version, 1);
        assert!(!config.background_enabled);
        assert!(!config.launch_at_login);
        assert!(manager.path().exists());
    }

    #[test]
    fn saves_and_loads_configuration() {
        let (_directory, manager) = create_test_manager();

        let mut config = RuntimeConfig {
            schema_version: 1,
            background_enabled: true,
            launch_at_login: false,
            permission_states: BTreeMap::new(),
            ..RuntimeConfig::default()
        };

        config
            .permission_states
            .insert(Permission::HardwareMetadata, PermissionState::Granted);

        config
            .permission_states
            .insert(Permission::NetworkMetadata, PermissionState::Denied);
        manager.save(&config).unwrap();

        let loaded = manager.load().unwrap();

        assert_eq!(
            loaded.permission_states.get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
        assert_eq!(
            loaded.permission_states.get(&Permission::NetworkMetadata),
            Some(&PermissionState::Denied)
        );
        assert_eq!(loaded.schema_version, 1);
        assert!(loaded.background_enabled);
        assert!(!loaded.launch_at_login);
    }

    // Replaces `rejects_invalid_configuration`. Since Phase 2.6.4 an unusable
    // configuration is quarantined and replaced by the defaults instead of
    // making the load fail; the original is kept, so nothing is rejected
    // silently. Failure cases are covered below.
    #[test]
    fn invalid_configuration_is_quarantined_and_replaced_by_defaults() {
        let (_directory, manager) = create_test_manager();

        fs::write(manager.path(), "{ invalid json }").unwrap();

        let report = manager.load_with_report().unwrap();
        let recovery = report.diagnostics.recovery.expect("recovery expected");

        assert!(report.config.permission_states.is_empty());
        assert_eq!(
            fs::read_to_string(&recovery.quarantined_path).unwrap(),
            "{ invalid json }"
        );
        assert!(manager.load().unwrap().permission_states.is_empty());
    }

    const ROAMING_JSON: &str = r#"{
  "schema_version": 1,
  "background_enabled": true,
  "launch_at_login": true,
  "permission_states": {
    "hardware_metadata": "granted",
    "process_metadata": "denied"
  }
}"#;

    const LOCAL_JSON: &str = r#"{
  "schema_version": 1,
  "background_enabled": false,
  "launch_at_login": false,
  "permission_states": {
    "network_metadata": "granted"
  }
}"#;

    /// Creates separate Local and Roaming directories, as on Windows.
    fn create_directories() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();

        let local = directory.path().join("Local").join("atlas").join("config");
        let roaming = directory
            .path()
            .join("Roaming")
            .join("atlas")
            .join("config");
        fs::create_dir_all(&roaming).unwrap();

        (directory, local, roaming)
    }

    #[test]
    fn existing_local_configuration_takes_precedence_over_roaming() {
        let (_directory, local, roaming) = create_directories();
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("runtime.json"), LOCAL_JSON).unwrap();
        fs::write(roaming.join("runtime.json"), ROAMING_JSON).unwrap();

        let manager = ConfigManager::from_directories(&local, &roaming).unwrap();
        let config = manager.load().unwrap();

        assert!(!config.background_enabled);
        assert_eq!(
            config.permission_states.get(&Permission::NetworkMetadata),
            Some(&PermissionState::Granted)
        );
        assert!(config
            .permission_states
            .get(&Permission::HardwareMetadata)
            .is_none());
    }

    #[test]
    fn existing_local_configuration_is_never_overwritten() {
        let (_directory, local, roaming) = create_directories();
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("runtime.json"), LOCAL_JSON).unwrap();
        fs::write(roaming.join("runtime.json"), ROAMING_JSON).unwrap();

        ConfigManager::from_directories(&local, &roaming).unwrap();

        assert_eq!(
            fs::read_to_string(local.join("runtime.json")).unwrap(),
            LOCAL_JSON
        );
        assert_eq!(
            fs::read_to_string(roaming.join("runtime.json")).unwrap(),
            ROAMING_JSON
        );
    }

    #[test]
    fn roaming_configuration_migrates_when_local_is_absent() {
        let (_directory, local, roaming) = create_directories();
        fs::write(roaming.join("runtime.json"), ROAMING_JSON).unwrap();

        let manager = ConfigManager::from_directories(&local, &roaming).unwrap();

        assert_eq!(manager.path(), local.join("runtime.json"));
        assert_eq!(
            fs::read_to_string(local.join("runtime.json")).unwrap(),
            ROAMING_JSON
        );

        let config = manager.load().unwrap();

        assert!(config.background_enabled);
        assert!(config.launch_at_login);
    }

    #[test]
    fn migrated_permission_decisions_are_preserved() {
        let (_directory, local, roaming) = create_directories();
        fs::write(roaming.join("runtime.json"), ROAMING_JSON).unwrap();

        let manager = ConfigManager::from_directories(&local, &roaming).unwrap();
        let config = manager.load().unwrap();

        let mut expected = BTreeMap::new();
        expected.insert(Permission::HardwareMetadata, PermissionState::Granted);
        expected.insert(Permission::ProcessMetadata, PermissionState::Denied);

        assert_eq!(config.permission_states, expected);
    }

    #[test]
    fn runtime_host_restores_migrated_permissions() {
        use crate::runtime::RuntimeHost;

        let (_directory, local, roaming) = create_directories();
        fs::write(roaming.join("runtime.json"), ROAMING_JSON).unwrap();

        let manager = ConfigManager::from_directories(&local, &roaming).unwrap();
        let mut host = RuntimeHost::new(&manager).unwrap();
        host.start().unwrap();

        assert!(host.is_authorized(Permission::HardwareMetadata));
        assert_eq!(
            host.permission_state(Permission::ProcessMetadata),
            PermissionState::Denied
        );
        assert_eq!(
            host.permission_state(Permission::NetworkMetadata),
            PermissionState::NotRequested
        );
    }

    #[test]
    fn later_changes_are_saved_locally_and_roaming_is_left_alone() {
        use crate::runtime::RuntimeHost;

        let (_directory, local, roaming) = create_directories();
        fs::write(roaming.join("runtime.json"), ROAMING_JSON).unwrap();

        let manager = ConfigManager::from_directories(&local, &roaming).unwrap();
        let mut host = RuntimeHost::new(&manager).unwrap();
        host.revoke_permission(Permission::HardwareMetadata)
            .unwrap();
        drop(host);

        let local_config: RuntimeConfig =
            serde_json::from_str(&fs::read_to_string(local.join("runtime.json")).unwrap()).unwrap();

        assert!(local_config
            .permission_states
            .get(&Permission::HardwareMetadata)
            .is_none());
        assert_eq!(
            fs::read_to_string(roaming.join("runtime.json")).unwrap(),
            ROAMING_JSON
        );

        // A second start must keep using the Local file, not re-migrate.
        let manager = ConfigManager::from_directories(&local, &roaming).unwrap();
        let host = RuntimeHost::new(&manager).unwrap();

        assert!(!host.is_authorized(Permission::HardwareMetadata));
    }

    #[test]
    fn default_configuration_is_created_when_neither_location_exists() {
        let directory = tempfile::tempdir().unwrap();
        let local = directory.path().join("Local").join("atlas").join("config");
        let roaming = directory
            .path()
            .join("Roaming")
            .join("atlas")
            .join("config");

        let manager = ConfigManager::from_directories(&local, &roaming).unwrap();
        let config = manager.load().unwrap();

        assert_eq!(config.schema_version, 1);
        assert!(!config.background_enabled);
        assert!(!config.launch_at_login);
        assert!(config.permission_states.is_empty());
        assert!(local.join("runtime.json").exists());
        assert!(!roaming.exists());
    }

    #[test]
    fn migration_failure_is_reported_and_keeps_roaming_configuration() {
        let (_directory, local, roaming) = create_directories();
        fs::write(roaming.join("runtime.json"), "{ not valid json").unwrap();

        let error = match ConfigManager::from_directories(&local, &roaming) {
            Ok(_) => panic!("migration of an invalid configuration must fail"),
            Err(error) => error,
        };
        let message = error.to_string();

        assert!(message.contains("failed to migrate configuration"));
        assert!(message.contains("left unchanged"));
        assert!(!local.join("runtime.json").exists());
        assert_eq!(
            fs::read_to_string(roaming.join("runtime.json")).unwrap(),
            "{ not valid json"
        );
    }

    #[test]
    fn migration_failure_prevents_runtime_host_from_starting_with_defaults() {
        let (_directory, local, roaming) = create_directories();
        fs::write(roaming.join("runtime.json"), "{ not valid json").unwrap();

        // The manager cannot be created, so no host can be built on top of an
        // empty default configuration that would hide the user's old one.
        assert!(ConfigManager::from_directories(&local, &roaming).is_err());
        assert!(!local.join("runtime.json").exists());
    }

    #[test]
    fn same_local_and_legacy_directory_does_not_migrate() {
        let directory = tempfile::tempdir().unwrap();
        let shared = directory.path().join("atlas").join("config");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("runtime.json"), ROAMING_JSON).unwrap();

        let manager = ConfigManager::from_directories(&shared, &shared).unwrap();

        assert_eq!(fs::read_to_string(manager.path()).unwrap(), ROAMING_JSON);
        assert!(!manager.path().with_extension("json.migrating").exists());
    }

    #[test]
    fn valid_current_configuration_loads_without_diagnostics() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), ROAMING_JSON).unwrap();

        let report = manager.load_with_report().unwrap();

        assert!(report.config.background_enabled);
        assert_eq!(
            report
                .config
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
        assert_eq!(report.diagnostics, ConfigDiagnostics::default());
        assert!(quarantined_files(&manager).is_empty());
        assert_eq!(fs::read_to_string(manager.path()).unwrap(), ROAMING_JSON);
    }

    #[test]
    fn missing_configuration_creates_defaults_without_recovery() {
        let (_directory, manager) = create_test_manager();

        let report = manager.load_with_report().unwrap();

        assert_eq!(report.diagnostics, ConfigDiagnostics::default());
        assert!(report.config.permission_states.is_empty());
        assert!(manager.path().exists());
        assert!(quarantined_files(&manager).is_empty());
    }

    #[test]
    fn corrupt_json_is_quarantined_with_its_original_bytes() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), "{ this is not json").unwrap();

        let report = manager.load_with_report().unwrap();
        let recovery = report.diagnostics.recovery.expect("recovery expected");

        assert!(!recovery.reason.is_empty());
        assert_eq!(
            quarantined_files(&manager),
            vec![recovery.quarantined_path.clone()]
        );
        assert_eq!(
            fs::read(&recovery.quarantined_path).unwrap(),
            b"{ this is not json"
        );
    }

    #[test]
    fn truncated_json_is_quarantined_and_defaults_are_written() {
        let (_directory, manager) = create_test_manager();
        let truncated = &ROAMING_JSON[..ROAMING_JSON.len() / 2];
        fs::write(manager.path(), truncated).unwrap();

        let report = manager.load_with_report().unwrap();
        let recovery = report.diagnostics.recovery.expect("recovery expected");

        assert!(recovery.reason.contains("truncated"));
        assert_eq!(
            fs::read_to_string(&recovery.quarantined_path).unwrap(),
            truncated
        );
        assert!(report.config.permission_states.is_empty());
        assert!(manager.load().unwrap().permission_states.is_empty());
    }

    #[test]
    fn empty_file_is_quarantined() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), "").unwrap();

        let report = manager.load_with_report().unwrap();

        assert!(report.diagnostics.recovery.is_some());
        assert_eq!(quarantined_files(&manager).len(), 1);
    }

    #[test]
    fn byte_order_mark_configuration_loads_without_recovery() {
        let (_directory, manager) = create_test_manager();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(ROAMING_JSON.as_bytes());
        fs::write(manager.path(), &bytes).unwrap();

        let report = manager.load_with_report().unwrap();

        assert!(report.diagnostics.recovery.is_none());
        assert!(report.config.launch_at_login);
        assert_eq!(
            report
                .config
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
        assert!(quarantined_files(&manager).is_empty());
        // Loading does not rewrite a usable file.
        assert_eq!(fs::read(manager.path()).unwrap(), bytes);
    }

    #[test]
    fn invalid_permission_state_quarantines_the_file_and_grants_nothing() {
        let (_directory, manager) = create_test_manager();
        fs::write(
            manager.path(),
            r#"{"permission_states": {"hardware_metadata": "granted", "network_metadata": "allowed"}}"#,
        )
        .unwrap();

        let report = manager.load_with_report().unwrap();
        let recovery = report.diagnostics.recovery.expect("recovery expected");

        assert!(recovery.reason.contains("invalid state"));
        assert!(report.config.permission_states.is_empty());
        assert!(manager.load().unwrap().permission_states.is_empty());
    }

    #[test]
    fn missing_permission_map_loads_with_no_decisions() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), r#"{"schema_version": 1}"#).unwrap();

        let report = manager.load_with_report().unwrap();

        assert!(report.config.permission_states.is_empty());
        assert_eq!(report.diagnostics, ConfigDiagnostics::default());
    }

    #[test]
    fn null_permission_map_loads_with_a_warning_and_no_recovery() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), r#"{"permission_states": null}"#).unwrap();

        let report = manager.load_with_report().unwrap();

        assert!(report.config.permission_states.is_empty());
        assert!(report.diagnostics.recovery.is_none());
        assert_eq!(report.diagnostics.warnings.len(), 1);
        assert!(quarantined_files(&manager).is_empty());
    }

    #[test]
    fn unknown_permissions_and_fields_are_kept_and_written_back() {
        let (_directory, manager) = create_test_manager();
        fs::write(
            manager.path(),
            r#"{
              "schema_version": 1,
              "future_flag": {"a": [1, 2]},
              "permission_states": {
                "hardware_metadata": "granted",
                "camera_access": "granted"
              }
            }"#,
        )
        .unwrap();

        let report = manager.load_with_report().unwrap();

        assert!(report.diagnostics.recovery.is_none());
        assert_eq!(report.diagnostics.warnings.len(), 2);
        // The unknown permission is not one of the decisions this build acts on.
        assert_eq!(report.config.permission_states.len(), 1);

        manager.save(&report.config).unwrap();

        let written = fs::read_to_string(manager.path()).unwrap();
        assert!(written.contains("camera_access"));
        assert!(written.contains("future_flag"));

        let reloaded = manager.load().unwrap();
        assert_eq!(
            reloaded.unknown_fields["future_flag"],
            serde_json::json!({"a": [1, 2]})
        );
        assert_eq!(
            reloaded
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
    }

    #[test]
    fn future_schema_version_is_refused_and_the_file_is_left_alone() {
        let (_directory, manager) = create_test_manager();
        let newer =
            r#"{"schema_version": 2, "permission_states": {"hardware_metadata": "granted"}}"#;
        fs::write(manager.path(), newer).unwrap();

        let error = manager.load_with_report().unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("schema version 2"));
        assert!(error.to_string().contains("not modified"));
        assert_eq!(fs::read_to_string(manager.path()).unwrap(), newer);
        assert!(quarantined_files(&manager).is_empty());

        // Asking again does not change anything either.
        assert!(manager.load().is_err());
        assert_eq!(fs::read_to_string(manager.path()).unwrap(), newer);
    }

    #[test]
    fn defaults_after_recovery_grant_nothing_and_are_saved() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), "{ corrupt").unwrap();

        let report = manager.load_with_report().unwrap();

        assert_eq!(report.config.schema_version, 1);
        assert!(!report.config.background_enabled);
        assert!(!report.config.launch_at_login);
        assert!(report.config.permission_states.is_empty());
        assert!(report.config.unknown_permissions.is_empty());
        assert!(report.config.unknown_fields.is_empty());

        // The file on disk is now a valid default configuration.
        let saved = manager.load_with_report().unwrap();

        assert!(saved.diagnostics.recovery.is_none());
        assert!(saved.config.permission_states.is_empty());
        // Recovery happens once: the second load quarantines nothing new.
        assert_eq!(quarantined_files(&manager).len(), 1);
    }

    #[test]
    fn no_corrupt_configuration_can_grant_a_permission() {
        use crate::runtime::RuntimeHost;

        let corpus = [
            r#"{"permission_states": {"hardware_metadata": "granted", "network_metadata": "x"}}"#,
            r#"{"permission_states": {"hardware_metadata": "granted"}, "background_enabled": "yes"}"#,
            r#"{"permission_states": ["granted"]}"#,
            r#"{"permission_states": {"hardware_metadata": "granted""#,
            r#"{"schema_version": 0, "permission_states": {"hardware_metadata": "granted"}}"#,
            r#"{"schema_version": "1", "permission_states": {"hardware_metadata": "granted"}}"#,
            r#"["hardware_metadata", "granted"]"#,
            "granted granted granted",
        ];

        for body in corpus {
            let (_directory, manager) = create_test_manager();
            fs::write(manager.path(), body).unwrap();

            let host = RuntimeHost::new(&manager)
                .unwrap_or_else(|_| panic!("recovery expected for {body}"));

            for permission in ALL_PERMISSIONS {
                assert!(!host.is_authorized(permission), "{body}: {permission:?}");
                assert_eq!(
                    host.permission_state(permission),
                    PermissionState::NotRequested,
                    "{body}: {permission:?}"
                );
            }

            assert!(host.config_diagnostics().recovery.is_some(), "{body}");
            assert_eq!(quarantined_files(&manager).len(), 1, "{body}");
        }
    }

    #[test]
    fn repeated_corruption_keeps_every_original() {
        let (_directory, manager) = create_test_manager();

        fs::write(manager.path(), "{ first").unwrap();
        manager.load().unwrap();
        fs::write(manager.path(), "{ second").unwrap();
        manager.load().unwrap();

        let mut contents: Vec<String> = quarantined_files(&manager)
            .iter()
            .map(|path| fs::read_to_string(path).unwrap())
            .collect();
        contents.sort();

        assert_eq!(contents, vec!["{ first", "{ second"]);
    }

    #[test]
    fn quarantine_failure_is_reported_and_nothing_is_replaced() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), "{ corrupt").unwrap();

        let error = manager
            .load_with_report_using(|_path| {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"))
            })
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("could not be set aside"));
        assert!(error.to_string().contains("locked"));
        assert!(error.to_string().contains("left in place"));
        assert_eq!(fs::read_to_string(manager.path()).unwrap(), "{ corrupt");
        assert!(quarantined_files(&manager).is_empty());
    }

    #[test]
    fn failure_to_save_defaults_after_quarantine_names_the_kept_original() {
        let directory = tempfile::tempdir().unwrap();
        let storage = directory.path().join("storage");
        fs::create_dir(&storage).unwrap();
        let manager = ConfigManager::new_for_test(storage.join("runtime.json"));
        fs::write(manager.path(), "{ corrupt").unwrap();
        let moved = directory.path().join("moved-away.json");

        let error = manager
            .load_with_report_using(|path| {
                // The original is safe, then storage disappears.
                fs::rename(path, &moved)?;
                fs::remove_dir(path.parent().unwrap())?;
                Ok(moved.clone())
            })
            .unwrap_err();

        assert!(error.to_string().contains("was kept as"));
        assert!(error.to_string().contains("moved-away.json"));
        assert!(error.to_string().contains("could not be saved"));
        assert_eq!(fs::read_to_string(&moved).unwrap(), "{ corrupt");
    }

    #[test]
    fn unreadable_configuration_is_an_error_and_is_not_quarantined() {
        let (_directory, manager) = create_test_manager();
        // A directory where the file should be cannot be read as a file.
        fs::create_dir(manager.path()).unwrap();

        let error = manager.load_with_report().unwrap_err();

        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("could not read configuration"));
        assert!(manager.path().is_dir());
        assert!(quarantined_files(&manager).is_empty());
    }

    #[test]
    fn load_and_load_with_report_agree() {
        let (_directory, manager) = create_test_manager();
        fs::write(manager.path(), ROAMING_JSON).unwrap();

        let plain = manager.load().unwrap();
        let report = manager.load_with_report().unwrap();

        assert_eq!(plain.permission_states, report.config.permission_states);
        assert_eq!(plain.background_enabled, report.config.background_enabled);
    }

    #[test]
    fn real_directories_keep_the_same_layout_and_filename() {
        let base = directories::ProjectDirs::from("com", "atlas", "AtlasLayoutTest").unwrap();

        // Same application sub-path under both bases; only the base differs.
        assert_eq!(
            base.config_local_dir().file_name(),
            base.config_dir().file_name()
        );

        #[cfg(not(windows))]
        assert_eq!(base.config_local_dir(), base.config_dir());

        #[cfg(windows)]
        {
            assert_ne!(base.config_local_dir(), base.config_dir());
            assert!(base
                .config_local_dir()
                .to_string_lossy()
                .to_lowercase()
                .contains("appdata\\local"));
            assert!(base
                .config_dir()
                .to_string_lossy()
                .to_lowercase()
                .contains("appdata\\roaming"));
        }
    }

    #[test]
    fn lock_path_is_next_to_the_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));

        assert_eq!(manager.lock_path(), directory.path().join("runtime.lock"));
    }

    #[test]
    fn read_current_does_not_create_or_change_anything() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));

        assert!(manager.read_current().unwrap().is_none());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);

        std::fs::write(manager.path(), "{ broken").unwrap();
        let error = manager.read_current().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read_to_string(manager.path()).unwrap(), "{ broken");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn read_current_refuses_a_newer_schema() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));
        std::fs::write(manager.path(), r#"{"schema_version": 99}"#).unwrap();

        let error = manager.read_current().unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn read_current_returns_a_valid_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));
        std::fs::write(manager.path(), r#"{"background_enabled": true}"#).unwrap();

        assert!(manager.read_current().unwrap().unwrap().background_enabled);
    }
}
