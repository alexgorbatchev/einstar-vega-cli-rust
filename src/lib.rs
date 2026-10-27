//! Read-only scanner workflows, an interactive browser, and an offline mock.

pub mod cli;
pub mod mock;
pub mod output;
pub mod signals;
pub mod ui;
pub mod workflow;

/// Failures retain their class until the executable chooses an exit status.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Arguments(#[from] clap::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Vega(#[from] einstar_vega::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("project {name:?} not found; available projects: {available}")]
    MissingProject { name: String, available: String },
    #[error("downloaded {completed} files; failures: {failures}")]
    Partial { completed: usize, failures: String },
    #[error("{summary} before {cause}")]
    Download {
        summary: workflow::Summary,
        cause: Box<Error>,
    },
    #[error("cancelled")]
    Cancelled,
    #[error("background worker: {0}")]
    Worker(#[from] tokio::task::JoinError),
    #[error("terminal restoration failed after {cause}: {restore}")]
    Restoration {
        cause: Box<Error>,
        restore: std::io::Error,
    },
    #[error("{cause}; shutdown failed: {shutdown}")]
    Shutdown {
        cause: Box<Error>,
        shutdown: Box<Error>,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
