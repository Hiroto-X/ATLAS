use std::fmt;

#[derive(Debug)]
pub enum RuntimeError {
    InvalidState(String),
    InitializationFailed(String),
    ShutdownFailed(String),
    ConfigurationFailed(String),
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
        }
    }
}

impl std::error::Error for RuntimeError {}