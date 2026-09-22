//! Driver and protocol error types.

/// Errors produced by a driver operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DriverError {
    #[error("device is unreachable: {0}")]
    Unreachable(String),
    #[error("device request timed out after {0} ms")]
    Timeout(u64),
    #[error("device returned a malformed response: {0}")]
    MalformedResponse(String),
    #[error("device refused the command: {0}")]
    Refused(String),
    #[error("this driver does not support the requested operation")]
    UnsupportedOperation,
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}
