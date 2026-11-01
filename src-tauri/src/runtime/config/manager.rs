use std::{
    fs,
    io,
    path::{Path, PathBuf},
};


use super::schema::RuntimeConfig;


pub struct ConfigManager {
    config_path: PathBuf,
}

impl ConfigManager {
    pub fn new(app_name: &str) -> io::Result<Self> {
        let base_dir = directories::ProjectDirs::from(
            "com",
            "atlas",
            app_name,
        )
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Unable to locate application configuration directory",
            )
        })?;

        let config_dir = base_dir.config_dir();

        fs::create_dir_all(config_dir)?;

        Ok(Self {
            config_path: config_dir.join("runtime.json"),
        })
    }

    pub fn load(&self) -> io::Result<RuntimeConfig> {
        if !self.config_path.exists() {
            let config = RuntimeConfig::default();
            self.save(&config)?;
            return Ok(config);
        }

        let contents = fs::read_to_string(&self.config_path)?;

        serde_json::from_str(&contents).map_err(|error| {
            io::Error::new(io::ErrorKind::InvalidData, error)
        })
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

fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temp_path = path.with_extension("json.tmp");

    fs::write(&temp_path, contents)?;
    
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    Ok(())
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

        config.permission_states.insert(
            Permission::HardwareMetadata,
            PermissionState::Granted,
        );
        
        config.permission_states.insert(
            Permission::NetworkMetadata,
            PermissionState::Denied,
        );
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
}