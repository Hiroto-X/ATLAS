#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeStatus {
    Starting,
    Running,
    Paused,
    ShuttingDown,
    Stopped,
}