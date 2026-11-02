use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::atomic::atomic_write;
use super::migration::migrate_legacy_config;
use super::schema::RuntimeConfig;

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

    pub fn load(&self) -> io::Result<RuntimeConfig> {
        if !self.config_path.exists() {
            let config = RuntimeConfig::default();
            self.save(&config)?;
            return Ok(config);
        }

        let contents = fs::read_to_string(&self.config_path)?;

        serde_json::from_str(&contents)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn save(&self, config: &RuntimeConfig) -> io::Result<()> {
        let contents = serde_json::to_string_pretty(config)?;

        atomic_write(&self.config_path, contents.as_bytes())
    }

    pub fn path(&self) -> &Path {
        &self.config_path
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

    #[test]
    fn rejects_invalid_configuration() {
        let (_directory, manager) = create_test_manager();

        fs::write(manager.path(), "{ invalid json }").unwrap();

        let result = manager.load();

        assert!(result.is_err());
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
        let host = RuntimeHost::new(&manager).unwrap();

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
}
