`einstar-vega` reads scanner metadata and downloads project files for Rust applications that need direct access to an Einstar Vega without account registration.

# What It Does

- **Device inspection**: Reads device metadata while preserving binary, numeric, nested, and tagged CBOR values.
- **Project browsing**: Lists projects and builds trees with file identities independent of display labels.
- **Atomic downloads**: Verifies exact file lengths and preserves existing files when transfers fail or are cancelled.
- **Confined output**: Rejects traversal paths and symlinked output descendants.

# How It Works

1. Construct a client with the scanner's `host:port` address and a cancellation token.
2. Connect, read projects, and list each project's file paths.
3. Select files directly or through a project tree; overlapping selections return each file once.
4. Download files into an opened output directory, receiving progress after successful writes.

# How it Really Works

1. Calls use plaintext HTTP without authentication. The client disables proxies, redirects, automatic retries, and content decompression. Constructing it makes no network request and reads no application configuration variables.
2. Metadata requests have a 30-second deadline and a 16 MiB response limit by default. The handshake must be exactly `connect success`; file-size responses must contain exactly eight big-endian bytes. File transfers have no overall deadline and accept cancellation throughout network reads and output writes.
3. Each download uses a private sibling staging directory. Exact-length verification, flush, sync, and fallible close precede atomic replacement. Existing file permissions are retained. Earlier completed files remain available when another file fails; downloads do not resume partial transfers.
4. Cancel the token and await the operation to observe cleanup errors. `Error::Cleanup` includes a relative staging recovery path. `Error::Published` explicitly reports that replacement succeeded but staging cleanup failed. Dropping the future only attempts cleanup and cannot report its failures.
5. The output root is trusted caller configuration, including `.`. Remote paths must be absolute POSIX paths with no dot segments, repeated separators, NULs, or backslashes. A single trailing slash is normalized. Output descendants use retained directory handles without following symlinks.
6. Client clones share HTTP connections and can serve concurrent calls. Project trees keep state in memory; sizes remain unknown until all descendant file sizes are known. There is no metadata cache, import-time work, or telemetry. Callers choose and own the async runtime and cancellation lifetime.

# Prerequisites

- An Einstar Vega reachable over its HTTP interface, normally port 8080.
- A [Tokio](https://tokio.rs/) runtime with networking and filesystem support for async calls.

# Installation

This implementation crate belongs to the `einstar-vega-cli` repository. Rust packages and release artifacts are unpublished. From a consumer beside that repository:

```sh
cargo add einstar-vega --path ../einstar-vega-cli/crates/einstar-vega
```

# Quick Start

```rust,no_run
use einstar_vega::{CancellationToken, Client};

#[tokio::main]
async fn main() -> einstar_vega::Result<()> {
    let client = Client::new("192.168.30.3:8080")?;
    let token = CancellationToken::new();
    client.connect(&token).await?;
    for (name, project) in client.projects(&token).await? {
        println!("{name}: {} ({})", project.path, project.date_time);
    }
    Ok(())
}
```

# API

| Entry point | Behavior |
| :--- | :--- |
| `Client::new`, `Client::with_policy` | Validate a `host:port` or `[IPv6]:port` address and create a reusable client. |
| `Client::connect`, `device_info`, `projects`, `list_files`, `file_size` | Read and validate scanner responses asynchronously. |
| `Client::download` | Stream an exact declared length into any async writer; the caller owns partial bytes on failure. |
| `RemotePath::parse`, `relative_to` | Validate remote paths and resolve a nonempty file path under a project root. |
| `ProjectTree::new`, `files_for`, `set_size` | Construct a tree, deduplicate selections, and maintain checked aggregate sizes. Node IDs are scoped to their tree. |
| `DownloadSpec::new`, `OutputDirectory::open`, `download` | Validate a project-relative destination and publish a completed file atomically. |
| `CancellationToken`, `Error` | Control operation lifetimes and inspect typed failure or publication outcomes. |

# Configuration

| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `RequestPolicy::timeout` | `Duration` | 30 seconds | Whole metadata request deadline, including response reads; must be nonzero. |
| `RequestPolicy::metadata_limit` | `usize` | 16 MiB | Maximum encoded metadata response; must be nonzero. |

# Compatibility

Requires Rust 1.98 or newer. Local HTTP and filesystem tests run on Linux; physical-device behavior and Windows/macOS execution remain unverified. Windows filename semantics, including reserved device names and colon-delimited streams, have not been validated; the output-confinement checks are verified on Linux. See [Windows naming rules](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file) when evaluating another native target. The source Go tool reports testing firmware `v1.3.0-22`, which is not a Rust hardware verification claim.

Protocol behavior follows [alexgorbatchev/einstar-vega-cli](https://github.com/alexgorbatchev/einstar-vega-cli), which credits [rabits/einstar-vega-extract](https://github.com/rabits/einstar-vega-extract). The first list-result item remains opaque because the source does not establish its meaning.

# License

[MIT License](LICENSE), copyright 2026 Alex Gorbatchev.
