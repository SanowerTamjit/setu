use std::path::PathBuf;

/// Errors from reading, changing or saving the host inventory.
#[derive(Debug, thiserror::Error)]
pub enum HostsError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not save {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a valid hosts file: {message}")]
    Parse { path: PathBuf, message: String },
    #[error(
        "{path} was written by a newer version of Setu (format {found}); update Setu to change it"
    )]
    NewerFormat { path: PathBuf, found: u32 },
    #[error("{0}")]
    Invalid(String),
    #[error("there is already a host named `{0}`")]
    Duplicate(String),
    #[error("no host named `{0}`")]
    NotFound(String),
}
