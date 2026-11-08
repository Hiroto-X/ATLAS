pub mod runtime;

use runtime::{
    service::{IpcError, PermissionDto, RuntimeService, RuntimeStatusDto},
    Permission,
};
use serde::Serialize;
use tauri::{Manager, RunEvent, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemInfo {
    os: String,
    architecture: String,
    hostname: String,
    app_version: String,
}

#[tauri::command]
fn get_system_info() -> SystemInfo {
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Unknown device".to_string());

    SystemInfo {
        os: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        hostname,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

// The commands below only forward to `RuntimeService`, which holds the logic
// and is tested without Tauri.

#[tauri::command]
fn get_runtime_status(runtime: State<'_, RuntimeService>) -> RuntimeStatusDto {
    runtime.status()
}

#[tauri::command]
fn get_permissions(runtime: State<'_, RuntimeService>) -> Result<Vec<PermissionDto>, IpcError> {
    runtime.permissions()
}

#[tauri::command]
fn grant_permission(
    runtime: State<'_, RuntimeService>,
    permission: Permission,
) -> Result<PermissionDto, IpcError> {
    runtime.grant_permission(permission)
}

#[tauri::command]
fn deny_permission(
    runtime: State<'_, RuntimeService>,
    permission: Permission,
) -> Result<PermissionDto, IpcError> {
    runtime.deny_permission(permission)
}

#[tauri::command]
fn revoke_permission(
    runtime: State<'_, RuntimeService>,
    permission: Permission,
) -> Result<PermissionDto, IpcError> {
    runtime.revoke_permission(permission)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            // Registered exactly once. A runtime that cannot start (for
            // example because another ATLAS is running) is still registered,
            // and reports why through `get_runtime_status`.
            app.manage(RuntimeService::start_default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_system_info,
            get_runtime_status,
            get_permissions,
            grant_permission,
            deny_permission,
            revoke_permission
        ])
        .build(tauri::generate_context!())
        .expect("error while building ATLAS");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            if let Some(runtime) = app_handle.try_state::<RuntimeService>() {
                runtime.shutdown();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_info_returns_valid_os() {
        let info = get_system_info();

        assert!(!info.os.is_empty());
    }

    #[test]
    fn system_info_returns_valid_architecture() {
        let info = get_system_info();

        assert!(!info.architecture.is_empty());
    }

    #[test]
    fn system_info_returns_valid_hostname() {
        let info = get_system_info();

        assert!(!info.hostname.is_empty());
        assert_ne!(info.hostname, "Unknown device");
    }

    #[test]
    fn system_info_returns_app_version() {
        let info = get_system_info();

        assert!(!info.app_version.is_empty());
        assert_eq!(info.app_version, env!("CARGO_PKG_VERSION"));
    }
}
