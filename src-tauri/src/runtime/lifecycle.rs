use super::error::RuntimeError;
use super::state::RuntimeStatus;

pub struct LifecycleManager {
    status: RuntimeStatus,
}

impl LifecycleManager {
    pub fn new() -> Self {
        Self {
            status: RuntimeStatus::Starting,
        }
    }

    pub fn status(&self) -> &RuntimeStatus {
        &self.status
    }

    pub fn start(&mut self) -> Result<(), RuntimeError> {
        if self.status != RuntimeStatus::Starting {
            return Err(RuntimeError::InvalidState(
                "Runtime can only start from Starting state".into(),
            ));
        }

        self.status = RuntimeStatus::Running;

        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), RuntimeError> {
        if self.status != RuntimeStatus::Running {
            return Err(RuntimeError::InvalidState(
                "Only a running runtime can be paused".into(),
            ));
        }

        self.status = RuntimeStatus::Paused;

        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), RuntimeError> {
        if self.status != RuntimeStatus::Paused {
            return Err(RuntimeError::InvalidState(
                "Only a paused runtime can be resumed".into(),
            ));
        }

        self.status = RuntimeStatus::Running;

        Ok(())
    }

    pub fn begin_shutdown(&mut self) -> Result<(), RuntimeError> {
        if self.status != RuntimeStatus::Running
            && self.status != RuntimeStatus::Paused
        {
            return Err(RuntimeError::InvalidState(
                "Runtime cannot begin shutdown from its current state".into(),
            ));
        }

        self.status = RuntimeStatus::ShuttingDown;

        Ok(())
    }

    pub fn complete_shutdown(&mut self) -> Result<(), RuntimeError> {
        if self.status != RuntimeStatus::ShuttingDown {
            return Err(RuntimeError::InvalidState(
                "Runtime must be shutting down before it can stop".into(),
            ));
        }

        self.status = RuntimeStatus::Stopped;

        Ok(())
    }
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_successfully() {
        let mut manager = LifecycleManager::new();

        manager.start().unwrap();

        assert_eq!(*manager.status(), RuntimeStatus::Running);
    }

    #[test]
    fn pauses_successfully() {
        let mut manager = LifecycleManager::new();

        manager.start().unwrap();
        manager.pause().unwrap();

        assert_eq!(*manager.status(), RuntimeStatus::Paused);
    }

    #[test]
    fn resumes_successfully() {
        let mut manager = LifecycleManager::new();

        manager.start().unwrap();
        manager.pause().unwrap();
        manager.resume().unwrap();

        assert_eq!(*manager.status(), RuntimeStatus::Running);
    }

    #[test]
    fn shutdown_has_two_stages() {
        let mut manager = LifecycleManager::new();

        manager.start().unwrap();
        manager.begin_shutdown().unwrap();

        assert_eq!(*manager.status(), RuntimeStatus::ShuttingDown);

        manager.complete_shutdown().unwrap();

        assert_eq!(*manager.status(), RuntimeStatus::Stopped);
    }

    #[test]
    fn cannot_resume_running_runtime() {
        let mut manager = LifecycleManager::new();

        manager.start().unwrap();

        assert!(manager.resume().is_err());
    }

    #[test]
    fn cannot_pause_stopped_runtime() {
        let mut manager = LifecycleManager::new();

        manager.start().unwrap();
        manager.begin_shutdown().unwrap();
        manager.complete_shutdown().unwrap();

        assert!(manager.pause().is_err());
    }

    #[test]
    fn cannot_complete_shutdown_before_starting_it() {
        let mut manager = LifecycleManager::new();

        assert!(manager.complete_shutdown().is_err());
    }
}