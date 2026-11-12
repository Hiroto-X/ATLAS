//! End-to-end tests of the runtime.
//!
//! Each test drives `RuntimeService`, the same entry point the Tauri commands
//! use, against real files in a temporary directory. They check that the
//! parts built in Phase 2.6 work together: configuration loading, recovery,
//! migration, the single-instance lock, permission persistence, stale-writer
//! protection and the lifecycle rules.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use super::config::ConfigManager;
use super::permissions::{Permission, PermissionState};
use super::service::{PermissionDto, RuntimeService, RuntimeStateDto};

struct Fixture {
    directory: tempfile::TempDir,
    manager: ConfigManager,
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let manager = ConfigManager::new_for_test(directory.path().join("runtime.json"));

    Fixture { directory, manager }
}

fn dto(service: &RuntimeService, permission: Permission) -> PermissionDto {
    service
        .permissions()
        .unwrap()
        .into_iter()
        .find(|dto| dto.permission == permission)
        .unwrap()
}

fn file_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    names
}

fn quarantined(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("runtime.json.corrupt-")
        })
        .collect()
}

#[test]
fn fresh_install_runs_persists_and_restores_decisions() {
    let fixture = fixture();

    let first = RuntimeService::start(&fixture.manager);
    let status = first.status();
    assert_eq!(status.state, RuntimeStateDto::Running);
    assert!(status.notices.is_empty());
    assert!(first.permissions().unwrap().iter().all(|d| !d.authorized));

    first
        .grant_permission(Permission::HardwareMetadata)
        .unwrap();
    first
        .grant_permission(Permission::SoftwareInventory)
        .unwrap();
    first.deny_permission(Permission::NetworkMetadata).unwrap();
    first.shutdown();
    drop(first);

    let second = RuntimeService::start(&fixture.manager);
    assert!(dto(&second, Permission::HardwareMetadata).authorized);
    assert!(dto(&second, Permission::SoftwareInventory).authorized);
    assert_eq!(
        dto(&second, Permission::NetworkMetadata).state,
        PermissionState::Denied
    );
    assert_eq!(
        dto(&second, Permission::ProcessMetadata).state,
        PermissionState::NotRequested
    );

    let saved = fixture.manager.load().unwrap();
    assert_eq!(saved.schema_version, 1);
    assert_eq!(saved.permission_states.len(), 3);
}

#[test]
fn legacy_configuration_is_migrated_and_then_used_by_the_runtime() {
    let root = tempfile::tempdir().unwrap();
    let (local, legacy) = (root.path().join("local"), root.path().join("roaming"));
    fs::create_dir(&legacy).unwrap();
    let old = r#"{"permission_states": {"hardware_metadata": "granted"}}"#;
    fs::write(legacy.join("runtime.json"), old).unwrap();

    let manager = ConfigManager::from_directories(&local, &legacy).unwrap();
    let service = RuntimeService::start(&manager);

    assert_eq!(service.status().state, RuntimeStateDto::Running);
    assert!(dto(&service, Permission::HardwareMetadata).authorized);
    // The old file is kept, unchanged, and new decisions go to the new place.
    service
        .deny_permission(Permission::HardwareMetadata)
        .unwrap();
    assert_eq!(
        fs::read_to_string(legacy.join("runtime.json")).unwrap(),
        old
    );
    assert!(fs::read_to_string(local.join("runtime.json"))
        .unwrap()
        .contains("denied"));
}

#[test]
fn a_damaged_configuration_is_quarantined_then_the_runtime_works_normally() {
    let fixture = fixture();
    let damaged = r#"{"permission_states": {"hardware_metadata": "granted""#;
    fs::write(fixture.manager.path(), damaged).unwrap();

    let first = RuntimeService::start(&fixture.manager);
    let status = first.status();

    assert_eq!(status.state, RuntimeStateDto::Running);
    assert!(status.config_recovered);
    assert!(first.permissions().unwrap().iter().all(|d| !d.authorized));
    let set_aside = quarantined(fixture.directory.path());
    assert_eq!(set_aside.len(), 1);
    assert_eq!(fs::read_to_string(&set_aside[0]).unwrap(), damaged);

    first.grant_permission(Permission::ProcessMetadata).unwrap();
    drop(first);

    let second = RuntimeService::start(&fixture.manager);
    assert!(!second.status().config_recovered);
    assert!(dto(&second, Permission::ProcessMetadata).authorized);
    assert_eq!(quarantined(fixture.directory.path()).len(), 1);
}

#[test]
fn a_newer_configuration_is_never_touched_and_works_once_replaced() {
    let fixture = fixture();
    let newer = r#"{"schema_version": 2, "permission_states": {"hardware_metadata": "granted"}}"#;
    fs::write(fixture.manager.path(), newer).unwrap();

    let refused = RuntimeService::start(&fixture.manager);

    assert_eq!(refused.status().state, RuntimeStateDto::Unavailable);
    assert_eq!(
        refused.permissions().unwrap_err().code,
        "configuration_unavailable"
    );
    assert_eq!(
        refused
            .grant_permission(Permission::HardwareMetadata)
            .unwrap_err()
            .code,
        "configuration_unavailable"
    );
    assert_eq!(fs::read_to_string(fixture.manager.path()).unwrap(), newer);
    assert!(quarantined(fixture.directory.path()).is_empty());
    drop(refused);

    fs::write(fixture.manager.path(), "{}").unwrap();
    assert_eq!(
        RuntimeService::start(&fixture.manager).status().state,
        RuntimeStateDto::Running
    );
}

#[test]
fn unrecognised_entries_survive_every_kind_of_change_and_are_never_granted() {
    let fixture = fixture();
    fs::write(
        fixture.manager.path(),
        r#"{"future_setting": {"a": 1}, "permission_states": {"camera_access": "granted"}}"#,
    )
    .unwrap();

    let first = RuntimeService::start(&fixture.manager);
    first
        .grant_permission(Permission::HardwareMetadata)
        .unwrap();
    first.deny_permission(Permission::HardwareMetadata).unwrap();
    first
        .revoke_permission(Permission::HardwareMetadata)
        .unwrap();
    first.grant_permission(Permission::NetworkMetadata).unwrap();
    assert!(first
        .permissions()
        .unwrap()
        .iter()
        .all(|d| { d.permission == Permission::NetworkMetadata || !d.authorized }));
    drop(first);

    let text = fs::read_to_string(fixture.manager.path()).unwrap();
    assert!(text.contains("future_setting"));
    assert!(text.contains("camera_access"));
    let second = RuntimeService::start(&fixture.manager);
    assert_eq!(second.permissions().unwrap().len(), Permission::ALL.len());
}

#[test]
fn a_second_instance_is_refused_and_cannot_release_the_first_ones_lock() {
    let fixture = fixture();
    let first = RuntimeService::start(&fixture.manager);

    for _ in 0..3 {
        let refused = RuntimeService::start(&fixture.manager);
        assert_eq!(refused.status().error.unwrap().code, "already_running");
        drop(refused);
    }

    // Dropping refused instances did not free the lock.
    assert_eq!(
        RuntimeService::start(&fixture.manager).status().state,
        RuntimeStateDto::Unavailable
    );
    first
        .grant_permission(Permission::HardwareMetadata)
        .unwrap();
    drop(first);
    assert_eq!(
        RuntimeService::start(&fixture.manager).status().state,
        RuntimeStateDto::Running
    );
}

#[test]
fn losing_storage_is_fail_closed_through_the_whole_stack() {
    let directory = tempfile::tempdir().unwrap();
    let storage = directory.path().join("storage");
    fs::create_dir(&storage).unwrap();
    let manager = ConfigManager::new_for_test(storage.join("runtime.json"));
    let service = RuntimeService::start(&manager);
    service
        .grant_permission(Permission::HardwareMetadata)
        .unwrap();
    fs::remove_dir_all(&storage).unwrap();

    let denial = service.deny_permission(Permission::HardwareMetadata);
    let grant = service.grant_permission(Permission::NetworkMetadata);

    assert_eq!(denial.unwrap_err().code, "permission_not_persisted");
    assert_eq!(grant.unwrap_err().code, "configuration_failed");
    assert!(service.permissions().unwrap().iter().all(|d| !d.authorized));
    assert_eq!(
        dto(&service, Permission::HardwareMetadata).state,
        PermissionState::Denied
    );
}

#[test]
fn changes_made_outside_atlas_while_it_runs_never_grant_anything() {
    let fixture = fixture();
    let service = RuntimeService::start(&fixture.manager);
    service
        .grant_permission(Permission::HardwareMetadata)
        .unwrap();

    // Someone revokes a permission by editing the file: that is adopted.
    fs::write(
        fixture.manager.path(),
        r#"{"permission_states": {"hardware_metadata": "denied"}}"#,
    )
    .unwrap();
    service
        .grant_permission(Permission::SoftwareInventory)
        .unwrap();
    assert!(!dto(&service, Permission::HardwareMetadata).authorized);
    assert!(dto(&service, Permission::SoftwareInventory).authorized);

    // Someone grants one by editing the file: that is refused, and the file
    // is left alone.
    let external = r#"{"permission_states": {"process_metadata": "granted"}}"#;
    fs::write(fixture.manager.path(), external).unwrap();
    let error = service
        .grant_permission(Permission::NetworkMetadata)
        .unwrap_err();

    assert_eq!(error.code, "configuration_failed");
    assert!(!dto(&service, Permission::ProcessMetadata).authorized);
    assert!(!dto(&service, Permission::NetworkMetadata).authorized);
    assert_eq!(
        fs::read_to_string(fixture.manager.path()).unwrap(),
        external
    );
}

#[test]
fn shutdown_stops_everything_but_keeps_the_saved_decisions() {
    let fixture = fixture();
    let service = RuntimeService::start(&fixture.manager);
    service
        .grant_permission(Permission::HardwareMetadata)
        .unwrap();
    let before = fs::read_to_string(fixture.manager.path()).unwrap();

    service.shutdown();
    service.shutdown();

    assert_eq!(service.status().state, RuntimeStateDto::Stopped);
    assert!(service.permissions().unwrap().iter().all(|d| !d.authorized));
    assert_eq!(
        dto(&service, Permission::HardwareMetadata).state,
        PermissionState::Granted
    );
    for result in [
        service.grant_permission(Permission::NetworkMetadata),
        service.deny_permission(Permission::HardwareMetadata),
        service.revoke_permission(Permission::HardwareMetadata),
    ] {
        assert_eq!(result.unwrap_err().code, "invalid_state");
    }
    assert_eq!(fs::read_to_string(fixture.manager.path()).unwrap(), before);
}

#[test]
fn concurrent_commands_leave_memory_and_disk_in_agreement() {
    let fixture = fixture();
    let service = Arc::new(RuntimeService::start(&fixture.manager));

    let handles: Vec<_> = (0..6)
        .map(|worker| {
            let service = Arc::clone(&service);
            thread::spawn(move || {
                for round in 0..20 {
                    let permission = Permission::ALL[(worker + round) % Permission::ALL.len()];
                    match (worker + round) % 3 {
                        0 => service.grant_permission(permission).map(|_| ()),
                        1 => service.deny_permission(permission).map(|_| ()),
                        _ => service.revoke_permission(permission).map(|_| ()),
                    }
                    .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let saved = fixture.manager.load().unwrap();
    for current in service.permissions().unwrap() {
        let stored = saved
            .permission_states
            .get(&current.permission)
            .copied()
            .unwrap_or(PermissionState::NotRequested);

        assert_eq!(current.state, stored, "{:?}", current.permission);
        assert_eq!(current.authorized, stored == PermissionState::Granted);
    }
}

#[test]
fn no_error_or_status_shown_to_the_interface_reveals_file_locations() {
    let fixture = fixture();
    let location = fixture.directory.path().display().to_string();
    let mut shown = Vec::new();

    fs::write(fixture.manager.path(), r#"{"schema_version": 9}"#).unwrap();
    let unavailable = RuntimeService::start(&fixture.manager);
    shown.push(serde_json::to_string(&unavailable.status()).unwrap());
    shown.push(serde_json::to_string(&unavailable.permissions().unwrap_err()).unwrap());
    drop(unavailable);

    fs::write(fixture.manager.path(), "{ broken").unwrap();
    let recovered = RuntimeService::start(&fixture.manager);
    shown.push(serde_json::to_string(&recovered.status()).unwrap());
    let second = RuntimeService::start(&fixture.manager);
    shown.push(serde_json::to_string(&second.status()).unwrap());
    recovered.shutdown();
    shown.push(
        serde_json::to_string(
            &recovered
                .grant_permission(Permission::HardwareMetadata)
                .unwrap_err(),
        )
        .unwrap(),
    );

    for text in shown {
        assert!(!text.contains(&location), "{text}");
        assert!(!text.contains("runtime.json"), "{text}");
        assert!(!text.contains("corrupt"), "{text}");
    }
}

#[test]
fn only_the_expected_files_are_left_behind() {
    let fixture = fixture();
    let service = RuntimeService::start(&fixture.manager);
    service
        .grant_permission(Permission::HardwareMetadata)
        .unwrap();
    service
        .deny_permission(Permission::NetworkMetadata)
        .unwrap();
    service.shutdown();
    drop(service);

    assert_eq!(
        file_names(fixture.directory.path()),
        vec!["runtime.json", "runtime.lock"]
    );
}
