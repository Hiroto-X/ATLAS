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

    /// Whether the runtime is fully running. Paused and every other state
    /// are not: nothing may act on a permission outside `Running`.
    pub fn is_running(&self) -> bool {
        self.status == RuntimeStatus::Running
    }

    /// Whether permission decisions may still be changed. They may until
    /// shutdown begins.
    pub fn allows_permission_changes(&self) -> bool {
        matches!(
            self.status,
            RuntimeStatus::Starting | RuntimeStatus::Running | RuntimeStatus::Paused
        )
    }

    /// Begins shutdown from `Starting`, `Running` or `Paused`. Calling it
    /// again once shutdown has begun, or finished, does nothing.
    pub fn begin_shutdown(&mut self) -> Result<(), RuntimeError> {
        match self.status {
            RuntimeStatus::Starting | RuntimeStatus::Running | RuntimeStatus::Paused => {
                self.status = RuntimeStatus::ShuttingDown;

                Ok(())
            }
            RuntimeStatus::ShuttingDown | RuntimeStatus::Stopped => Ok(()),
        }
    }

    /// Finishes a shutdown that has begun. Calling it again once stopped does
    /// nothing. It cannot skip `begin_shutdown`.
    pub fn complete_shutdown(&mut self) -> Result<(), RuntimeError> {
        match self.status {
            RuntimeStatus::ShuttingDown => {
                self.status = RuntimeStatus::Stopped;

                Ok(())
            }
            RuntimeStatus::Stopped => Ok(()),
            RuntimeStatus::Starting | RuntimeStatus::Running | RuntimeStatus::Paused => {
                Err(RuntimeError::InvalidState(
                    "Runtime must be shutting down before it can stop".into(),
                ))
            }
        }
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

    #[test]
    fn shutdown_can_begin_from_starting_running_and_paused() {
        let mut starting = LifecycleManager::new();
        starting.begin_shutdown().unwrap();
        assert_eq!(*starting.status(), RuntimeStatus::ShuttingDown);

        let mut running = LifecycleManager::new();
        running.start().unwrap();
        running.begin_shutdown().unwrap();
        assert_eq!(*running.status(), RuntimeStatus::ShuttingDown);

        let mut paused = LifecycleManager::new();
        paused.start().unwrap();
        paused.pause().unwrap();
        paused.begin_shutdown().unwrap();
        assert_eq!(*paused.status(), RuntimeStatus::ShuttingDown);
    }

    #[test]
    fn shutdown_is_idempotent() {
        let mut manager = LifecycleManager::new();
        manager.start().unwrap();

        manager.begin_shutdown().unwrap();
        manager.begin_shutdown().unwrap();
        assert_eq!(*manager.status(), RuntimeStatus::ShuttingDown);

        manager.complete_shutdown().unwrap();
        manager.complete_shutdown().unwrap();
        manager.begin_shutdown().unwrap();
        assert_eq!(*manager.status(), RuntimeStatus::Stopped);
    }

    #[test]
    fn a_stopped_runtime_cannot_be_restarted_or_resumed() {
        let mut manager = LifecycleManager::new();
        manager.begin_shutdown().unwrap();
        manager.complete_shutdown().unwrap();

        assert!(manager.start().is_err());
        assert!(manager.resume().is_err());
        assert!(manager.pause().is_err());
        assert_eq!(*manager.status(), RuntimeStatus::Stopped);
    }

    #[test]
    fn a_failed_transition_leaves_the_state_unchanged() {
        let mut manager = LifecycleManager::new();

        assert!(manager.pause().is_err());
        assert!(manager.resume().is_err());
        assert!(manager.complete_shutdown().is_err());
        assert_eq!(*manager.status(), RuntimeStatus::Starting);
    }

    #[test]
    fn only_running_is_running_and_changes_stop_at_shutdown() {
        let mut manager = LifecycleManager::new();
        assert!(!manager.is_running());
        assert!(manager.allows_permission_changes());

        manager.start().unwrap();
        assert!(manager.is_running());
        assert!(manager.allows_permission_changes());

        manager.pause().unwrap();
        assert!(!manager.is_running());
        assert!(manager.allows_permission_changes());

        manager.begin_shutdown().unwrap();
        assert!(!manager.is_running());
        assert!(!manager.allows_permission_changes());

        manager.complete_shutdown().unwrap();
        assert!(!manager.is_running());
        assert!(!manager.allows_permission_changes());
    }
}
