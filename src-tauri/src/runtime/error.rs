use std::fmt;

#[derive(Debug)]
pub enum RuntimeError {
    InvalidState(String),
    InitializationFailed(String),
    ShutdownFailed(String),
    ConfigurationFailed(String),
    /// A permission was restricted (denied or revoked) in memory, but the
    /// change could not be saved. The restriction is still in effect.
    PermissionNotPersisted(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::InvalidState(message) => {
                write!(f, "Invalid runtime state: {message}")
            }

            RuntimeError::InitializationFailed(message) => {
                write!(f, "Runtime initialization failed: {message}")
            }

            RuntimeError::ShutdownFailed(message) => {
                write!(f, "Runtime shutdown failed: {message}")
            }

            RuntimeError::ConfigurationFailed(message) => {
                write!(f, "Runtime configuration failed: {message}")
            }

            RuntimeError::PermissionNotPersisted(message) => {
                write!(f, "Permission change not persisted: {message}")
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_not_persisted_has_its_own_message() {
        let error = RuntimeError::PermissionNotPersisted("disk full".into());

        assert_eq!(
            error.to_string(),
            "Permission change not persisted: disk full"
        );
    }

    #[test]
    fn permission_not_persisted_is_distinct_from_configuration_failed() {
        let not_persisted = RuntimeError::PermissionNotPersisted("x".into());
        let failed = RuntimeError::ConfigurationFailed("x".into());

        assert!(matches!(
            not_persisted,
            RuntimeError::PermissionNotPersisted(_)
        ));
        assert!(!matches!(
            not_persisted,
            RuntimeError::ConfigurationFailed(_)
        ));
        assert_ne!(not_persisted.to_string(), failed.to_string());
    }
}
