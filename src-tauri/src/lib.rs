use serde::Serialize;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_system_info])
        .run(tauri::generate_context!())
        .expect("error while running ATLAS");
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
