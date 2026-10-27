//! Read-only access to the Einstar Vega scanner's HTTP interface.
//!
//! All network operations accept a caller-owned cancellation token. Device
//! paths are absolute POSIX paths; local output paths retain native OS strings.
//!
//! ```no_run
//! use einstar_vega::{CancellationToken, Client, DownloadSpec, OutputDirectory};
//! use std::path::Path;
//!
//! # async fn example() -> einstar_vega::Result<()> {
//! let client = Client::new("192.168.30.3:8080")?;
//! let token = CancellationToken::new();
//! client.connect(&token).await?;
//! let output = OutputDirectory::open(Path::new("projects"))?;
//! for (name, project) in client.projects(&token).await? {
//!     for path in client.list_files(&project.path, &token).await? {
//!         let spec = DownloadSpec::new(&name, project.path.clone(), path)?;
//!         let relative = output.download(&client, &spec, &token, |_| {}).await?;
//!         println!("{}", relative.display());
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod client;
mod path;
mod publication;
mod tree;

pub use cbor4ii::core::Value;
pub use client::{Client, DeviceInfo, Progress, ProjectInfo, RequestPolicy};
pub use path::RemotePath;
pub use publication::{DownloadSpec, OutputDirectory};
pub use tokio_util::sync::CancellationToken;
pub use tree::{Node, NodeId, NodeKind, ProjectTree};

/// Recoverable protocol, transport, cancellation, and filesystem failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("operation cancelled")]
    Cancelled,
    #[error("invalid path or address: {0}")]
    InvalidPath(String),
    #[error("invalid device response: {0}")]
    Protocol(String),
    #[error("{endpoint} returned HTTP {status}")]
    Status { endpoint: &'static str, status: u16 },
    #[error("download length mismatch: expected {expected} bytes, received {received}")]
    Size { expected: u64, received: u64 },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{cause}; closing incomplete download: {close}")]
    Close {
        cause: Box<Error>,
        close: std::io::Error,
    },
    #[error("{cause}; removing incomplete download at {staging:?}: {cleanup}")]
    Cleanup {
        cause: Box<Error>,
        staging: std::path::PathBuf,
        cleanup: std::io::Error,
    },
    #[error("download published at {path:?}; removing staging directory {staging:?}: {cleanup}")]
    Published {
        path: std::path::PathBuf,
        staging: std::path::PathBuf,
        cleanup: std::io::Error,
    },
}

/// Result returned by library operations.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn check_cancel(token: &CancellationToken) -> Result<()> {
    if token.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
