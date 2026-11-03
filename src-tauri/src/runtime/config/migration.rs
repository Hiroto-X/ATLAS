//! One-time migration of `runtime.json` from the legacy Roaming location to
//! the Local location.
//!
//! This is deliberately separate from ordinary configuration loading
//! (`ConfigManager::load`). It only ever copies: the legacy file is never
//! modified or deleted, and an existing destination is never overwritten.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::validation::parse_config_bytes;

/// Suffix of the temporary file used while migrating. It is intentionally not
/// `.json`, so a half-written copy can never be mistaken for a configuration.
const MIGRATION_TEMP_SUFFIX: &str = "migrating";

/// What `migrate_legacy_config` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// Legacy and current locations are the same file (non-Windows platforms),
    /// so there is nothing to migrate.
    SamePath,
    /// A configuration already exists at the current location. It was left
    /// untouched.
    CurrentConfigExists,
    /// There is no legacy configuration to migrate.
    NoLegacyConfig,
    /// The legacy configuration was copied to the current location.
    Migrated,
}

/// Copies the legacy configuration to `current_path` if, and only if, there is
/// no configuration at `current_path` yet.
///
/// * An existing `current_path` always wins and is never overwritten.
/// * The legacy file is never modified or deleted.
/// * The copy is written to a temporary file next to `current_path` and moved
///   into place last, so `current_path` either holds the complete migrated
///   configuration or does not exist.
/// * The legacy file must be a valid configuration. If it is not, or if any
///   step fails, an error naming both files is returned and nothing is left at
///   `current_path`. The legacy file is kept for the user.
///
/// The destination directory must already exist.
pub fn migrate_legacy_config(
    legacy_path: &Path,
    current_path: &Path,
) -> io::Result<MigrationOutcome> {
    if legacy_path == current_path {
        return Ok(MigrationOutcome::SamePath);
    }

    if file_exists(current_path)? {
        return Ok(MigrationOutcome::CurrentConfigExists);
    }

    if !file_exists(legacy_path)? {
        return Ok(MigrationOutcome::NoLegacyConfig);
    }

    let temp_path = temp_path_for(current_path);

    let result = copy_validated(legacy_path, current_path, &temp_path);

    if result.is_err() {
        // Never leave a partial copy behind. Failing to remove it is not
        // worth masking the original error.
        let _ = fs::remove_file(&temp_path);
    }

    result.map(|()| MigrationOutcome::Migrated)
}

fn copy_validated(legacy_path: &Path, current_path: &Path, temp_path: &Path) -> io::Result<()> {
    let contents = fs::read(legacy_path)
        .map_err(|error| migration_error(legacy_path, current_path, "read", error))?;

    // Validate before writing anything, using the same rules as loading, so a
    // file the loader accepts (for example one with a byte order mark) can be
    // migrated. The original bytes are copied as they are, so no value
    // (including permission decisions) is altered.
    parse_config_bytes(&contents).map_err(|error| {
        migration_error(
            legacy_path,
            current_path,
            "validate",
            io::Error::new(io::ErrorKind::InvalidData, error),
        )
    })?;

    fs::write(temp_path, &contents)
        .map_err(|error| migration_error(legacy_path, current_path, "write", error))?;

    // Re-check right before the move so a configuration created in the
    // meantime is never replaced.
    if file_exists(current_path)? {
        return Err(migration_error(
            legacy_path,
            current_path,
            "finish",
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a configuration appeared at the destination during migration",
            ),
        ));
    }

    fs::rename(temp_path, current_path)
        .map_err(|error| migration_error(legacy_path, current_path, "finish", error))
}

/// Treats "not found" as `false`, and any other failure to look at the path as
/// an error. Unknown is never treated as absent, so a file that cannot be
/// inspected is not overwritten or skipped by mistake.
fn file_exists(path: &Path) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("unable to inspect {}: {error}", path.display()),
        )),
    }
}

fn temp_path_for(current_path: &Path) -> PathBuf {
    let mut name = current_path.as_os_str().to_os_string();
    name.push(".");
    name.push(MIGRATION_TEMP_SUFFIX);

    PathBuf::from(name)
}

fn migration_error(
    legacy_path: &Path,
    current_path: &Path,
    step: &str,
    error: io::Error,
) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "failed to migrate configuration (could not {step}) from {} to {}: {error}. \
             The original configuration was left unchanged.",
            legacy_path.display(),
            current_path.display(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::config::RuntimeConfig;
    use crate::runtime::permissions::{Permission, PermissionState};
    use std::collections::BTreeMap;

    const LEGACY_JSON: &str = r#"{
  "schema_version": 1,
  "background_enabled": true,
  "launch_at_login": false,
  "permission_states": {
    "hardware_metadata": "granted",
    "network_metadata": "denied"
  }
}"#;

    /// Creates `legacy/` and `current/` directories in a temp directory.
    fn locations() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();

        let legacy_dir = directory.path().join("roaming");
        let current_dir = directory.path().join("local");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::create_dir_all(&current_dir).unwrap();

        (
            directory,
            legacy_dir.join("runtime.json"),
            current_dir.join("runtime.json"),
        )
    }

    fn temp_file_exists(current_path: &Path) -> bool {
        temp_path_for(current_path).exists()
    }

    #[test]
    fn migrates_legacy_configuration_when_current_is_absent() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();

        let outcome = migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(outcome, MigrationOutcome::Migrated);
        assert!(current.exists());
        assert!(!temp_file_exists(&current));
    }

    #[test]
    fn migrated_configuration_is_byte_identical_and_preserves_permissions() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();

        migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(fs::read_to_string(&current).unwrap(), LEGACY_JSON);

        let migrated: RuntimeConfig =
            serde_json::from_str(&fs::read_to_string(&current).unwrap()).unwrap();

        let mut expected = BTreeMap::new();
        expected.insert(Permission::HardwareMetadata, PermissionState::Granted);
        expected.insert(Permission::NetworkMetadata, PermissionState::Denied);

        assert_eq!(migrated.permission_states, expected);
        assert!(migrated.background_enabled);
        assert!(!migrated.launch_at_login);
        assert_eq!(migrated.schema_version, 1);
    }

    #[test]
    fn legacy_configuration_is_kept_after_migration() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();

        migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(fs::read_to_string(&legacy).unwrap(), LEGACY_JSON);
    }

    #[test]
    fn existing_current_configuration_is_never_overwritten() {
        let (_directory, legacy, current) = locations();
        let current_json = r#"{"schema_version":1,"background_enabled":false}"#;
        fs::write(&legacy, LEGACY_JSON).unwrap();
        fs::write(&current, current_json).unwrap();

        let outcome = migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(outcome, MigrationOutcome::CurrentConfigExists);
        assert_eq!(fs::read_to_string(&current).unwrap(), current_json);
        assert_eq!(fs::read_to_string(&legacy).unwrap(), LEGACY_JSON);
        assert!(!temp_file_exists(&current));
    }

    #[test]
    fn existing_current_configuration_wins_even_when_legacy_is_invalid() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, "{ not json").unwrap();
        fs::write(&current, LEGACY_JSON).unwrap();

        let outcome = migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(outcome, MigrationOutcome::CurrentConfigExists);
        assert_eq!(fs::read_to_string(&current).unwrap(), LEGACY_JSON);
    }

    #[test]
    fn nothing_happens_when_neither_configuration_exists() {
        let (_directory, legacy, current) = locations();

        let outcome = migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(outcome, MigrationOutcome::NoLegacyConfig);
        assert!(!current.exists());
        assert!(!legacy.exists());
        assert!(!temp_file_exists(&current));
    }

    #[test]
    fn same_path_is_not_migrated() {
        let (_directory, _legacy, current) = locations();
        fs::write(&current, LEGACY_JSON).unwrap();

        let outcome = migrate_legacy_config(&current, &current).unwrap();

        assert_eq!(outcome, MigrationOutcome::SamePath);
        assert_eq!(fs::read_to_string(&current).unwrap(), LEGACY_JSON);
    }

    #[test]
    fn migration_is_one_time() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();

        assert_eq!(
            migrate_legacy_config(&legacy, &current).unwrap(),
            MigrationOutcome::Migrated
        );

        // Later changes to the current file must survive a second run, even
        // though the legacy file is still there.
        let changed = r#"{"schema_version":1,"background_enabled":false}"#;
        fs::write(&current, changed).unwrap();

        assert_eq!(
            migrate_legacy_config(&legacy, &current).unwrap(),
            MigrationOutcome::CurrentConfigExists
        );
        assert_eq!(fs::read_to_string(&current).unwrap(), changed);
    }

    #[test]
    fn invalid_legacy_configuration_fails_clearly_and_leaves_no_destination() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, "{ not valid json").unwrap();

        let error = migrate_legacy_config(&legacy, &current).unwrap_err();
        let message = error.to_string();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(message.contains("failed to migrate configuration"));
        assert!(message.contains("validate"));
        assert!(message.contains(&legacy.display().to_string()));
        assert!(message.contains(&current.display().to_string()));
        assert!(message.contains("left unchanged"));
        assert!(!current.exists());
        assert!(!temp_file_exists(&current));
        assert_eq!(fs::read_to_string(&legacy).unwrap(), "{ not valid json");
    }

    #[test]
    fn unreadable_legacy_configuration_fails_clearly() {
        let (_directory, legacy, current) = locations();
        // A directory at the legacy path exists but cannot be read as a file.
        fs::create_dir(&legacy).unwrap();

        let error = migrate_legacy_config(&legacy, &current).unwrap_err();
        let message = error.to_string();

        assert!(message.contains("failed to migrate configuration"));
        assert!(message.contains("read"));
        assert!(message.contains(&legacy.display().to_string()));
        assert!(!current.exists());
        assert!(!temp_file_exists(&current));
        assert!(legacy.is_dir());
    }

    #[test]
    fn write_failure_is_reported_and_leaves_no_destination() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();
        // A directory in the way of the temporary file makes the write fail.
        fs::create_dir(temp_path_for(&current)).unwrap();

        let error = migrate_legacy_config(&legacy, &current).unwrap_err();
        let message = error.to_string();

        assert!(message.contains("failed to migrate configuration"));
        assert!(message.contains("write"));
        assert!(message.contains(&legacy.display().to_string()));
        assert!(message.contains(&current.display().to_string()));
        assert!(!current.exists());
        assert_eq!(fs::read_to_string(&legacy).unwrap(), LEGACY_JSON);
    }

    #[test]
    fn missing_destination_directory_is_reported_and_keeps_legacy() {
        let (directory, legacy, _current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();
        let current = directory.path().join("does-not-exist").join("runtime.json");

        let error = migrate_legacy_config(&legacy, &current).unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to migrate configuration"));
        assert!(!current.exists());
        assert_eq!(fs::read_to_string(&legacy).unwrap(), LEGACY_JSON);
    }

    #[test]
    fn failed_migration_can_be_retried_successfully() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();
        fs::create_dir(temp_path_for(&current)).unwrap();

        assert!(migrate_legacy_config(&legacy, &current).is_err());

        fs::remove_dir(temp_path_for(&current)).unwrap();

        assert_eq!(
            migrate_legacy_config(&legacy, &current).unwrap(),
            MigrationOutcome::Migrated
        );
        assert_eq!(fs::read_to_string(&current).unwrap(), LEGACY_JSON);
    }

    #[test]
    fn stale_temporary_file_from_an_earlier_attempt_is_replaced() {
        let (_directory, legacy, current) = locations();
        fs::write(&legacy, LEGACY_JSON).unwrap();
        fs::write(temp_path_for(&current), "leftover from a crash").unwrap();

        migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(fs::read_to_string(&current).unwrap(), LEGACY_JSON);
        assert!(!temp_file_exists(&current));
    }

    #[test]
    fn byte_order_mark_legacy_configuration_migrates_unchanged() {
        let (_directory, legacy, current) = locations();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(LEGACY_JSON.as_bytes());
        fs::write(&legacy, &bytes).unwrap();

        let outcome = migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(outcome, MigrationOutcome::Migrated);
        assert_eq!(fs::read(&current).unwrap(), bytes);
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
    }

    #[test]
    fn legacy_configuration_with_unknown_entries_migrates_unchanged() {
        let (_directory, legacy, current) = locations();
        let json = r#"{"schema_version":1,"future_flag":true,"permission_states":{"camera_access":"granted"}}"#;
        fs::write(&legacy, json).unwrap();

        let outcome = migrate_legacy_config(&legacy, &current).unwrap();

        assert_eq!(outcome, MigrationOutcome::Migrated);
        assert_eq!(fs::read_to_string(&current).unwrap(), json);
    }

    #[test]
    fn legacy_configuration_from_a_newer_version_is_not_migrated() {
        let (_directory, legacy, current) = locations();
        let newer = r#"{"schema_version": 2}"#;
        fs::write(&legacy, newer).unwrap();

        let error = migrate_legacy_config(&legacy, &current).unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to migrate configuration"));
        assert!(error.to_string().contains("schema version 2"));
        assert!(!current.exists());
        assert!(!temp_file_exists(&current));
        assert_eq!(fs::read_to_string(&legacy).unwrap(), newer);
    }

    #[test]
    fn legacy_configuration_with_an_invalid_permission_state_is_not_migrated() {
        let (_directory, legacy, current) = locations();
        let body = r#"{"permission_states": {"hardware_metadata": "allowed"}}"#;
        fs::write(&legacy, body).unwrap();

        let error = migrate_legacy_config(&legacy, &current).unwrap_err();

        assert!(error.to_string().contains("validate"));
        assert!(error.to_string().contains("invalid state"));
        assert!(!current.exists());
        assert_eq!(fs::read_to_string(&legacy).unwrap(), body);
    }

    #[test]
    fn temporary_file_name_is_not_a_json_configuration() {
        let temp = temp_path_for(Path::new("dir/runtime.json"));

        assert_eq!(temp, PathBuf::from("dir/runtime.json.migrating"));
        assert_ne!(temp.extension().and_then(|e| e.to_str()), Some("json"));
    }
}
