//! Validation of `runtime.json` contents.
//!
//! This turns raw bytes into a `RuntimeConfig`, or says why they cannot be
//! trusted. It never touches the file system.
//!
//! # Policy
//!
//! | Input | Result |
//! |---|---|
//! | UTF-8 byte order mark | Ignored. Windows tools often add one. |
//! | Invalid UTF-8, invalid or truncated JSON, empty file | `Malformed` |
//! | Top level is not a JSON object | `Malformed` |
//! | `schema_version` missing | Treated as version 1 (files from before the field was checked) |
//! | `schema_version` zero, negative, fractional or not a number | `Malformed` |
//! | `schema_version` newer than this build supports | `UnsupportedSchemaVersion` |
//! | `background_enabled` / `launch_at_login` not a boolean | `Malformed` |
//! | `permission_states` missing | No decisions: every permission is `NotRequested` |
//! | `permission_states` is `null` | Same as missing, with a warning |
//! | `permission_states` is not an object | `Malformed` |
//! | Known permission with a value that is not a valid state | `Malformed` |
//! | Permission name this build does not know | Preserved, never granted, with a warning |
//! | Top-level field this build does not know | Preserved |
//!
//! # Decisions
//!
//! * **`schema_version` is checked first**, before any other field, because a
//!   newer build may have changed what the other fields look like. A newer
//!   file is reported as unsupported rather than as corrupt, so it is never
//!   quarantined or replaced.
//! * **Unknown fields and unknown permission names are preserved, not
//!   rejected.** Rejecting them would treat a valid file from a slightly newer
//!   build as corrupt, and dropping them on the next save would silently
//!   discard data. They are kept verbatim and written back. An unknown
//!   permission is never interpreted, so it cannot authorize anything.
//! * **A file that cannot be interpreted safely is rejected as a whole**
//!   (`Malformed`), never partly salvaged. Salvaging would mean guessing which
//!   decisions to trust; rejecting the whole file means deny-by-default
//!   applies to everything. The caller keeps the original by quarantining it.
//! * **Duplicate keys** in a JSON object are resolved the way the JSON parser
//!   resolves them: the last one wins.

use serde_json::{Map, Value};
use std::{collections::BTreeMap, fmt};

use super::schema::{RuntimeConfig, CURRENT_SCHEMA_VERSION};
use crate::runtime::permissions::{Permission, PermissionState};

const UTF8_BYTE_ORDER_MARK: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Version assumed for a file that has no `schema_version` field.
const LEGACY_SCHEMA_VERSION: u32 = 1;

/// Longest piece of a bad value quoted in an error message.
const MAX_QUOTED_VALUE_LENGTH: usize = 40;

/// Why a configuration could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigParseError {
    /// The contents cannot be interpreted safely. The file can be quarantined
    /// and replaced by the default configuration.
    Malformed(String),
    /// The file was written by a newer version of ATLAS. It must not be
    /// modified, quarantined or replaced.
    UnsupportedSchemaVersion { found: u64, supported: u32 },
}

impl fmt::Display for ConfigParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigParseError::Malformed(reason) => write!(f, "{reason}"),
            ConfigParseError::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "configuration schema version {found} is newer than the newest version \
                 this build supports ({supported}); update ATLAS to use this configuration"
            ),
        }
    }
}

impl std::error::Error for ConfigParseError {}

/// A validated configuration, plus anything worth telling the user about.
#[derive(Debug, Clone)]
pub struct ParsedConfig {
    pub config: RuntimeConfig,
    pub warnings: Vec<String>,
}

/// Validates the raw contents of `runtime.json`.
pub fn parse_config_bytes(bytes: &[u8]) -> Result<ParsedConfig, ConfigParseError> {
    let bytes = bytes.strip_prefix(UTF8_BYTE_ORDER_MARK).unwrap_or(bytes);

    let text = std::str::from_utf8(bytes)
        .map_err(|error| malformed(format!("file is not valid UTF-8: {error}")))?;

    let value: Value = serde_json::from_str(text).map_err(|error| {
        if error.is_eof() {
            malformed(format!("file is incomplete or truncated: {error}"))
        } else {
            malformed(format!("file is not valid JSON: {error}"))
        }
    })?;

    config_from_value(value)
}

/// Validates an already parsed JSON value.
pub fn config_from_value(value: Value) -> Result<ParsedConfig, ConfigParseError> {
    let Value::Object(mut fields) = value else {
        return Err(malformed(format!(
            "top level must be a JSON object, found {}",
            describe(&value)
        )));
    };

    // First, before anything else can be misread.
    let schema_version = take_schema_version(&mut fields)?;

    let background_enabled = take_bool(&mut fields, "background_enabled")?;
    let launch_at_login = take_bool(&mut fields, "launch_at_login")?;

    let mut warnings = Vec::new();

    let (permission_states, unknown_permissions) = take_permissions(&mut fields, &mut warnings)?;

    // Anything left over is not understood by this build, and is kept.
    let unknown_fields: BTreeMap<String, Value> = fields.into_iter().collect();

    if !unknown_fields.is_empty() {
        let names: Vec<&str> = unknown_fields.keys().map(String::as_str).collect();

        warnings.push(format!(
            "unrecognised configuration field(s) were preserved: {}",
            names.join(", ")
        ));
    }

    Ok(ParsedConfig {
        config: RuntimeConfig {
            schema_version,
            background_enabled,
            launch_at_login,
            permission_states,
            unknown_permissions,
            unknown_fields,
        },
        warnings,
    })
}

fn take_schema_version(fields: &mut Map<String, Value>) -> Result<u32, ConfigParseError> {
    let Some(value) = fields.remove("schema_version") else {
        return Ok(LEGACY_SCHEMA_VERSION);
    };

    match value.as_u64() {
        None | Some(0) => Err(malformed(format!(
            "schema_version must be a positive whole number, found {}",
            quote(&value)
        ))),
        Some(found) if found > u64::from(CURRENT_SCHEMA_VERSION) => {
            Err(ConfigParseError::UnsupportedSchemaVersion {
                found,
                supported: CURRENT_SCHEMA_VERSION,
            })
        }
        // At most CURRENT_SCHEMA_VERSION, so it fits.
        Some(version) => Ok(version as u32),
    }
}

fn take_bool(fields: &mut Map<String, Value>, name: &str) -> Result<bool, ConfigParseError> {
    match fields.remove(name) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(value),
        Some(other) => Err(malformed(format!(
            "{name} must be true or false, found {}",
            quote(&other)
        ))),
    }
}

type PermissionTables = (
    BTreeMap<Permission, PermissionState>,
    BTreeMap<String, Value>,
);

fn take_permissions(
    fields: &mut Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Result<PermissionTables, ConfigParseError> {
    let entries = match fields.remove("permission_states") {
        None => return Ok(Default::default()),
        Some(Value::Null) => {
            warnings.push(
                "permission_states was null and was treated as having no permission decisions"
                    .to_string(),
            );
            return Ok(Default::default());
        }
        Some(Value::Object(entries)) => entries,
        Some(other) => {
            return Err(malformed(format!(
                "permission_states must be an object, found {}",
                describe(&other)
            )))
        }
    };

    let mut known = BTreeMap::new();
    let mut unknown = BTreeMap::new();

    for (name, value) in entries {
        match serde_json::from_value::<Permission>(Value::String(name.clone())) {
            Ok(permission) => {
                let state =
                    serde_json::from_value::<PermissionState>(value.clone()).map_err(|_| {
                        malformed(format!(
                            "invalid state {} for permission '{name}' \
                             (expected \"not_requested\", \"granted\" or \"denied\")",
                            quote(&value)
                        ))
                    })?;

                known.insert(permission, state);
            }
            Err(_) => {
                warnings.push(format!(
                    "unrecognised permission '{name}' was preserved and is not granted"
                ));
                unknown.insert(name, value);
            }
        }
    }

    Ok((known, unknown))
}

fn malformed(reason: String) -> ConfigParseError {
    ConfigParseError::Malformed(reason)
}

/// A short JSON-ish rendering of a value for error messages.
fn quote(value: &Value) -> String {
    let text = value.to_string();

    if text.chars().count() > MAX_QUOTED_VALUE_LENGTH {
        let shortened: String = text.chars().take(MAX_QUOTED_VALUE_LENGTH).collect();
        format!("{shortened}…")
    } else {
        text
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ParsedConfig, ConfigParseError> {
        parse_config_bytes(text.as_bytes())
    }

    fn parsed(text: &str) -> ParsedConfig {
        parse(text).unwrap_or_else(|error| panic!("expected valid configuration: {error}"))
    }

    fn malformed_reason(text: &str) -> String {
        match parse(text) {
            Err(ConfigParseError::Malformed(reason)) => reason,
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    const VALID: &str = r#"{
  "schema_version": 1,
  "background_enabled": true,
  "launch_at_login": false,
  "permission_states": {
    "hardware_metadata": "granted",
    "network_metadata": "denied"
  }
}"#;

    #[test]
    fn valid_current_configuration_parses() {
        let result = parsed(VALID);

        assert_eq!(result.config.schema_version, 1);
        assert!(result.config.background_enabled);
        assert!(!result.config.launch_at_login);
        assert_eq!(
            result
                .config
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
        assert_eq!(
            result
                .config
                .permission_states
                .get(&Permission::NetworkMetadata),
            Some(&PermissionState::Denied)
        );
        assert!(result.config.unknown_permissions.is_empty());
        assert!(result.config.unknown_fields.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn empty_object_gives_the_default_configuration() {
        let result = parsed("{}");

        assert_eq!(result.config.schema_version, 1);
        assert!(!result.config.background_enabled);
        assert!(!result.config.launch_at_login);
        assert!(result.config.permission_states.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn missing_schema_version_is_treated_as_version_one() {
        let result = parsed(r#"{"background_enabled": true}"#);

        assert_eq!(result.config.schema_version, 1);
        assert!(result.config.background_enabled);
    }

    #[test]
    fn missing_permission_map_means_no_decisions() {
        let result = parsed(r#"{"schema_version": 1, "background_enabled": false}"#);

        assert!(result.config.permission_states.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn null_permission_map_means_no_decisions_and_warns() {
        let result = parsed(r#"{"schema_version": 1, "permission_states": null}"#);

        assert!(result.config.permission_states.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("permission_states was null"));
    }

    #[test]
    fn non_object_permission_map_is_malformed() {
        for body in ["[]", "\"granted\"", "5", "true", "[\"hardware_metadata\"]"] {
            let reason = malformed_reason(&format!(r#"{{"permission_states": {body}}}"#));

            assert!(
                reason.contains("permission_states must be an object"),
                "{body}: {reason}"
            );
        }
    }

    #[test]
    fn explicit_not_requested_state_is_valid() {
        let result = parsed(r#"{"permission_states": {"hardware_metadata": "not_requested"}}"#);

        assert_eq!(
            result
                .config
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::NotRequested)
        );
    }

    #[test]
    fn unknown_permission_names_are_preserved_and_never_granted() {
        let result = parsed(
            r#"{"permission_states": {
                "hardware_metadata": "granted",
                "camera_access": "granted",
                "future_thing": {"nested": [1, 2]}
            }}"#,
        );

        // Known decisions are untouched.
        assert_eq!(
            result
                .config
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
        // Unknown ones are kept verbatim, but never become known decisions.
        assert_eq!(result.config.permission_states.len(), 1);
        assert_eq!(result.config.unknown_permissions.len(), 2);
        assert_eq!(
            result.config.unknown_permissions["camera_access"],
            Value::String("granted".into())
        );
        assert_eq!(
            result.config.unknown_permissions["future_thing"],
            serde_json::json!({"nested": [1, 2]})
        );
        assert_eq!(result.warnings.len(), 2);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("camera_access") && warning.contains("not granted")));
    }

    #[test]
    fn permission_names_are_case_sensitive() {
        let result = parsed(r#"{"permission_states": {"Hardware_Metadata": "granted"}}"#);

        assert!(result.config.permission_states.is_empty());
        assert!(result
            .config
            .unknown_permissions
            .contains_key("Hardware_Metadata"));
    }

    #[test]
    fn invalid_permission_states_are_malformed() {
        for body in [
            "\"allowed\"",
            "\"GRANTED\"",
            "\"\"",
            "null",
            "true",
            "1",
            "[]",
            "{\"state\": \"granted\"}",
        ] {
            let reason = malformed_reason(&format!(
                r#"{{"permission_states": {{"hardware_metadata": {body}}}}}"#
            ));

            assert!(reason.contains("invalid state"), "{body}: {reason}");
            assert!(reason.contains("hardware_metadata"), "{body}: {reason}");
        }
    }

    #[test]
    fn one_invalid_state_rejects_the_whole_file() {
        // The granted entry must not survive next to an invalid one.
        let result = parse(
            r#"{"permission_states": {
                "hardware_metadata": "granted",
                "network_metadata": "allowed"
            }}"#,
        );

        assert!(matches!(result, Err(ConfigParseError::Malformed(_))));
    }

    #[test]
    fn utf8_byte_order_mark_is_ignored() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(VALID.as_bytes());

        let result = parse_config_bytes(&bytes).unwrap();

        assert_eq!(
            result
                .config
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::Granted)
        );
        assert!(result.config.background_enabled);
    }

    #[test]
    fn byte_order_mark_alone_is_malformed() {
        let result = parse_config_bytes(&[0xEF, 0xBB, 0xBF]);

        assert!(matches!(result, Err(ConfigParseError::Malformed(_))));
    }

    #[test]
    fn utf16_file_is_malformed_not_misread() {
        // What older Windows editors write for "Unicode".
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "{}".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }

        let result = parse_config_bytes(&bytes);

        assert!(matches!(result, Err(ConfigParseError::Malformed(_))));
    }

    #[test]
    fn invalid_utf8_is_malformed() {
        let result = parse_config_bytes(&[b'{', 0xFF, 0xFE, b'}']);

        match result {
            Err(ConfigParseError::Malformed(reason)) => assert!(reason.contains("UTF-8")),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn empty_and_blank_files_are_malformed() {
        for body in ["", " ", "\n\n", "\t \r\n"] {
            let reason = malformed_reason(body);

            assert!(reason.contains("truncated"), "{body:?}: {reason}");
        }
    }

    #[test]
    fn truncated_json_is_malformed_and_says_so() {
        for cut in [
            r#"{"schema_version": 1, "permission_st"#,
            r#"{"schema_version": 1,"#,
            r#"{"schema_version": 1, "permission_states": {"hardware_metadata": "gran"#,
            "{",
        ] {
            let reason = malformed_reason(cut);

            assert!(reason.contains("truncated"), "{cut}: {reason}");
        }
    }

    #[test]
    fn every_prefix_of_a_valid_file_is_rejected_or_valid_never_granting_extra() {
        // Whatever a crash cuts the file down to, the result is either an
        // error, or a configuration that grants no more than the original.
        for end in 0..VALID.len() {
            if !VALID.is_char_boundary(end) {
                continue;
            }

            if let Ok(result) = parse(&VALID[..end]) {
                assert!(result
                    .config
                    .permission_states
                    .iter()
                    .all(|(permission, state)| {
                        *state != PermissionState::Granted
                            || *permission == Permission::HardwareMetadata
                    }));
            }
        }
    }

    #[test]
    fn invalid_json_is_malformed() {
        for body in [
            "{ invalid json }",
            "not json at all",
            "{\"a\": }",
            "{'single': 'quotes'}",
            "{\"a\": 1,}",
            "{\"a\": 1} trailing",
        ] {
            let reason = malformed_reason(body);

            assert!(!reason.is_empty(), "{body}");
        }
    }

    #[test]
    fn top_level_must_be_an_object() {
        for body in ["[]", "null", "5", "\"text\"", "true", "[{}]"] {
            let reason = malformed_reason(body);

            assert!(
                reason.contains("top level must be a JSON object"),
                "{body}: {reason}"
            );
        }
    }

    #[test]
    fn future_schema_versions_are_unsupported_not_malformed() {
        for found in ["2", "99", "18446744073709551615"] {
            let result = parse(&format!(r#"{{"schema_version": {found}}}"#));

            match result {
                Err(ConfigParseError::UnsupportedSchemaVersion {
                    found: seen,
                    supported,
                }) => {
                    assert_eq!(seen.to_string(), found);
                    assert_eq!(supported, CURRENT_SCHEMA_VERSION);
                }
                other => panic!("expected UnsupportedSchemaVersion for {found}, got {other:?}"),
            }
        }
    }

    #[test]
    fn unsupported_schema_version_is_decided_before_other_fields_are_read() {
        // A newer build may lay the other fields out differently. The file
        // must be reported as newer, not as corrupt.
        for body in [
            r#"{"schema_version": 2, "permission_states": "different shape now"}"#,
            r#"{"schema_version": 2, "background_enabled": "yes"}"#,
            r#"{"schema_version": 2, "permission_states": {"hardware_metadata": {"v": 2}}}"#,
        ] {
            assert!(
                matches!(
                    parse(body),
                    Err(ConfigParseError::UnsupportedSchemaVersion { .. })
                ),
                "{body}"
            );
        }
    }

    #[test]
    fn invalid_schema_versions_are_malformed() {
        for body in ["0", "-1", "1.5", "1.0", "\"1\"", "null", "true", "[]", "{}"] {
            let reason = malformed_reason(&format!(r#"{{"schema_version": {body}}}"#));

            assert!(
                reason.contains("schema_version must be"),
                "{body}: {reason}"
            );
        }
    }

    #[test]
    fn boolean_fields_must_be_booleans() {
        for field in ["background_enabled", "launch_at_login"] {
            for body in ["\"true\"", "1", "null", "[]"] {
                let reason = malformed_reason(&format!(r#"{{"{field}": {body}}}"#));

                assert!(reason.contains(field), "{field} {body}: {reason}");
            }
        }
    }

    #[test]
    fn unknown_top_level_fields_are_preserved() {
        let result = parsed(r#"{"schema_version": 1, "future_flag": true, "theme": {"dark": 1}}"#);

        assert_eq!(
            result.config.unknown_fields["future_flag"],
            Value::Bool(true)
        );
        assert_eq!(
            result.config.unknown_fields["theme"],
            serde_json::json!({"dark": 1})
        );
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("future_flag") && warning.contains("theme")));
    }

    #[test]
    fn unknown_fields_and_permissions_survive_a_save_and_reload() {
        let original = parsed(
            r#"{
              "schema_version": 1,
              "future_flag": [1, 2, 3],
              "permission_states": {
                "hardware_metadata": "denied",
                "camera_access": "granted"
              }
            }"#,
        );

        let written = serde_json::to_string_pretty(&original.config).unwrap();
        let reloaded = parse(&written).unwrap();

        assert_eq!(
            reloaded.config.unknown_fields["future_flag"],
            serde_json::json!([1, 2, 3])
        );
        assert_eq!(
            reloaded.config.unknown_permissions["camera_access"],
            Value::String("granted".into())
        );
        assert_eq!(
            reloaded
                .config
                .permission_states
                .get(&Permission::HardwareMetadata),
            Some(&PermissionState::Denied)
        );
    }

    #[test]
    fn serialized_configuration_keeps_the_established_field_order() {
        let text = serde_json::to_string_pretty(&RuntimeConfig::default()).unwrap();
        let version = text.find("schema_version").unwrap();
        let background = text.find("background_enabled").unwrap();
        let login = text.find("launch_at_login").unwrap();
        let permissions = text.find("permission_states").unwrap();

        assert!(version < background && background < login && login < permissions);
    }

    #[test]
    fn serialization_never_writes_a_known_field_twice() {
        let mut config = RuntimeConfig::default();
        config
            .unknown_fields
            .insert("schema_version".into(), Value::from(7));

        let text = serde_json::to_string(&config).unwrap();

        assert_eq!(text.matches("schema_version").count(), 1);
    }

    #[test]
    fn deserializing_runtime_config_uses_the_same_validation() {
        let config: RuntimeConfig = serde_json::from_str(VALID).unwrap();
        assert!(config.background_enabled);

        assert!(serde_json::from_str::<RuntimeConfig>("[]").is_err());
        assert!(serde_json::from_str::<RuntimeConfig>(
            r#"{"permission_states": {"hardware_metadata": "allowed"}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<RuntimeConfig>(r#"{"schema_version": 2}"#).is_err());
    }

    #[test]
    fn no_malformed_input_can_yield_a_configuration() {
        // Every one of these mentions "granted". None may produce a
        // configuration, because producing one would risk keeping the grant.
        for body in [
            r#"{"permission_states": {"hardware_metadata": "granted", "network_metadata": "x"}}"#,
            r#"{"permission_states": {"hardware_metadata": "granted"}, "background_enabled": "yes"}"#,
            r#"{"permission_states": ["granted"]}"#,
            r#"{"permission_states": {"hardware_metadata": "granted"}"#,
            r#"{"schema_version": 0, "permission_states": {"hardware_metadata": "granted"}}"#,
            r#"{"schema_version": "1", "permission_states": {"hardware_metadata": "granted"}}"#,
            r#"["hardware_metadata", "granted"]"#,
        ] {
            assert!(parse(body).is_err(), "{body}");
        }
    }

    #[test]
    fn error_messages_are_readable() {
        let unsupported = ConfigParseError::UnsupportedSchemaVersion {
            found: 5,
            supported: 1,
        };

        assert!(unsupported.to_string().contains("schema version 5"));
        assert!(unsupported.to_string().contains("(1)"));
        assert_eq!(ConfigParseError::Malformed("bad".into()).to_string(), "bad");
    }

    #[test]
    fn long_bad_values_are_shortened_in_messages() {
        let long = "x".repeat(500);
        let reason = malformed_reason(&format!(
            r#"{{"permission_states": {{"hardware_metadata": "{long}"}}}}"#
        ));

        assert!(reason.len() < 250, "{}", reason.len());
        assert!(reason.contains('…'));
    }
}
