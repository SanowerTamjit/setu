use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    /// The workflow breaks a rule; the message says which, for the person.
    #[error("{0}")]
    Invalid(String),
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{path} was written by a newer Setu (format {found}); update Setu to open it")]
    NewerFormat { path: PathBuf, found: u32 },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("no workflow called `{0}`")]
    NotFound(String),
}

pub(crate) fn invalid(message: impl Into<String>) -> WorkflowError {
    WorkflowError::Invalid(message.into())
}
