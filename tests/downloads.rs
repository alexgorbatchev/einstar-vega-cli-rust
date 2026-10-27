use axum::{
    Router,
    body::Bytes,
    http::StatusCode,
    routing::{get, post},
};
use einstar_vega::{CancellationToken, Client, DownloadSpec, RemotePath};
use einstar_vega_cli::{
    Error, cli,
    workflow::{self, DownloadEvent},
};

fn temporary() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}
fn job(file: &str) -> DownloadSpec {
    DownloadSpec::new(
        "Project",
        RemotePath::parse("/Project").unwrap(),
        RemotePath::parse(format!("/Project/{file}")).unwrap(),
    )
    .unwrap()
}
async fn server(
    truncate_second: bool,
) -> (
    Client,
    u16,
    CancellationToken,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let router = Router::new()
        .route("/connect", get(|| async { "connect success" }))
        .route("/request/projectsInfo", get(|| async { cbor4ii::serde::to_vec(Vec::new(), &serde_json::json!({"result":{"Project":{"Path":"/Project","DateTime":"today"}}})).unwrap() }))
        .route("/request/listFilePaths", post(|| async { cbor4ii::serde::to_vec(Vec::new(), &serde_json::json!({"result":[0,["/Project/a","/Project/b"]]})).unwrap() }))
        .route(
            "/file/info",
            post(|| async { 4_u64.to_be_bytes().to_vec() }),
        )
        .route(
            "/file/content",
            post(move |bytes: Bytes| async move {
                let request: serde_json::Value = cbor4ii::serde::from_slice(&bytes).unwrap();
                if truncate_second && request["path"] == "/Project/b" {
                    (StatusCode::OK, vec![1, 2])
                } else {
                    (StatusCode::OK, vec![1, 2, 3, 4])
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let client = Client::new(&listener.local_addr().unwrap().to_string()).unwrap();
    let token = CancellationToken::new();
    let shutdown = token.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    });
    (client, port, token, task)
}

#[tokio::test]
async fn failed_exact_length_keeps_existing_file_and_retains_previous_publication() {
    let (client, _, shutdown, server) = server(true).await;
    let root = temporary();
    std::fs::create_dir(root.path().join("Project")).unwrap();
    std::fs::write(root.path().join("Project/b"), "original").unwrap();
    let mut events = Vec::new();
    let summary = workflow::download(
        &client,
        root.path(),
        &[job("a"), job("b")],
        &CancellationToken::new(),
        |event| {
            events.push(event);
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (summary.completed, summary.published, summary.failures.len()),
        (1, 1, 1)
    );
    assert_eq!(
        std::fs::read(root.path().join("Project/a")).unwrap(),
        [1, 2, 3, 4]
    );
    assert_eq!(
        std::fs::read(root.path().join("Project/b")).unwrap(),
        b"original"
    );
    assert!(
        events.iter().any(
            |event| matches!(event, DownloadEvent::Failed { error, .. } if error.contains("2"))
        )
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("Project"))
            .unwrap()
            .count(),
        2
    );
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn output_failure_after_publish_returns_typed_partial_outcome() {
    let (client, _, shutdown, server) = server(false).await;
    let root = temporary();
    let error = workflow::download(
        &client,
        root.path(),
        &[job("a"), job("b")],
        &CancellationToken::new(),
        |event| {
            if matches!(event, DownloadEvent::Published(_)) {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(())
            }
        },
    )
    .await
    .unwrap_err();
    assert_eq!(cli::exit_status(&error), 6);
    let Error::Download { summary, cause } = error else {
        panic!("missing publication state")
    };
    assert_eq!((summary.completed, summary.published), (1, 1));
    assert!(matches!(*cause, Error::Io(_)));
    assert_eq!(
        std::fs::read(root.path().join("Project/a")).unwrap(),
        [1, 2, 3, 4]
    );
    assert!(!root.path().join("Project/b").exists());
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn cancellation_and_progress_write_failure_await_cleanup_and_preserve_counts() {
    let (client, _, shutdown, server) = server(false).await;
    let root = temporary();
    let token = CancellationToken::new();
    let error = workflow::download(
        &client,
        root.path(),
        &[job("a"), job("b")],
        &token,
        |event| {
            if matches!(event, DownloadEvent::Published(_)) {
                token.cancel();
            }
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(cli::exit_status(&error), 130);
    let Error::Download { summary, .. } = error else {
        panic!("missing publication state")
    };
    assert_eq!((summary.completed, summary.published), (1, 1));
    let error = workflow::download(
        &client,
        root.path(),
        &[job("b")],
        &CancellationToken::new(),
        |_| Err(std::io::ErrorKind::BrokenPipe.into()),
    )
    .await
    .unwrap_err();
    assert_eq!(cli::exit_status(&error), 1);
    assert!(!root.path().join("Project/b").exists());
    assert_eq!(
        std::fs::read_dir(root.path().join("Project"))
            .unwrap()
            .count(),
        1
    );
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn failed_event_output_retains_failure_and_no_false_publication() {
    let (client, _, shutdown, server) = server(true).await;
    let root = temporary();
    let error = workflow::download(
        &client,
        root.path(),
        &[job("b")],
        &CancellationToken::new(),
        |event| {
            if matches!(event, DownloadEvent::Failed { .. }) {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(())
            }
        },
    )
    .await
    .unwrap_err();
    let Error::Download { summary, .. } = error else {
        panic!("missing failure state")
    };
    assert_eq!(
        (summary.completed, summary.published, summary.failures.len()),
        (0, 0, 1)
    );
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn cli_reports_partial_file_failures_in_both_modes_and_preserves_successful_files() {
    let (_, port, shutdown, task) = server(true).await;
    let port = port.to_string();
    for mode in [cli_command_kit::Mode::Agent, cli_command_kit::Mode::Human] {
        let root = temporary();
        let mut out = Vec::new();
        let error = cli::execute(
            [
                "vega-cli",
                "project",
                "download",
                "Project",
                "--ip",
                "127.0.0.1",
                "--port",
                &port,
                "--output",
                root.path().to_str().unwrap(),
            ],
            mode,
            &CancellationToken::new(),
            &mut out,
        )
        .await
        .unwrap_err();
        assert_eq!(cli::exit_status(&error), 6);
        assert!(matches!(error, Error::Partial { completed: 1, .. }));
        let text = String::from_utf8(out).unwrap();
        if mode == cli_command_kit::Mode::Agent {
            assert!(text.contains("progress:"));
            assert!(text.contains("published:"));
            assert!(text.contains("failed:"));
            assert!(text.contains("downloaded:1 published:1 failed:1"));
        } else {
            assert!(text.contains("[INFO]"));
            assert!(text.contains("[OK] Downloaded"));
            assert!(text.contains("[WARN] Could not finish"));
            assert!(text.contains("1 files; 1 published; 1 failed"));
        }
        assert_eq!(
            std::fs::read(root.path().join("Project/a")).unwrap(),
            [1, 2, 3, 4]
        );
        assert!(!root.path().join("Project/b").exists());
    }
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn final_summary_write_failure_retains_two_published_files() {
    struct FailSummary(Vec<u8>);
    impl std::io::Write for FailSummary {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.extend_from_slice(bytes);
            if self
                .0
                .windows(b"downloaded:".len())
                .any(|bytes| bytes == b"downloaded:")
            {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let (_, port, shutdown, task) = server(false).await;
    let port = port.to_string();
    let root = temporary();
    let error = cli::execute(
        [
            "vega-cli",
            "project",
            "download",
            "--all",
            "--ip",
            "127.0.0.1",
            "--port",
            &port,
            "--output",
            root.path().to_str().unwrap(),
        ],
        cli_command_kit::Mode::Agent,
        &CancellationToken::new(),
        &mut FailSummary(Vec::new()),
    )
    .await
    .unwrap_err();
    assert_eq!(cli::exit_status(&error), 6);
    assert!(
        matches!(&error,Error::Download{summary,..} if summary.completed==2 && summary.published==2)
    );
    assert_eq!(
        std::fs::read(root.path().join("Project/b")).unwrap(),
        [1, 2, 3, 4]
    );
    let wrapped = Error::Restoration {
        cause: Box::new(error),
        restore: std::io::ErrorKind::BrokenPipe.into(),
    };
    assert_eq!(cli::exit_status(&wrapped), 6);
    let joined = Error::Shutdown {
        cause: Box::new(Error::Cancelled),
        shutdown: Box::new(wrapped),
    };
    assert_eq!(cli::exit_status(&joined), 6);
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn progress_output_failure_preserves_independent_cleanup_failure_and_recovery_path() {
    use std::os::unix::fs::PermissionsExt;
    let (client, _, shutdown, server) = server(false).await;
    let root = temporary();
    let mut staging = None;
    let error = workflow::download(
        &client,
        root.path(),
        &[job("a")],
        &CancellationToken::new(),
        |event| {
            if matches!(event, DownloadEvent::Progress { .. }) {
                let path = std::fs::read_dir(root.path().join("Project"))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".vega-download-")
                    })
                    .unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
                staging = Some(path);
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            Ok(())
        },
    )
    .await
    .unwrap_err();
    let staging = staging.unwrap();
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(staging.exists());
    assert!(!root.path().join("Project/a").exists());
    let message = error.to_string();
    assert!(
        message.contains("broken pipe; shutdown failed:"),
        "{message}"
    );
    assert!(
        message.contains("removing incomplete download"),
        "{message}"
    );
    assert!(
        message.contains(staging.file_name().unwrap().to_str().unwrap()),
        "{message}"
    );
    let Error::Download { cause, summary } = error else {
        panic!("lost download state");
    };
    assert_eq!(summary.failures.len(), 1);
    let Error::Shutdown {
        cause,
        shutdown: cleanup,
    } = *cause
    else {
        panic!("lost independent cleanup error");
    };
    assert!(
        matches!(*cause, Error::Io(ref error) if error.kind() == std::io::ErrorKind::BrokenPipe)
    );
    assert!(matches!(
        *cleanup,
        Error::Vega(einstar_vega::Error::Cleanup { .. })
    ));
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn cancellation_during_progress_preserves_existing_file_and_reports_cancellation() {
    let (client, _, shutdown, server) = server(false).await;
    let root = temporary();
    std::fs::create_dir(root.path().join("Project")).unwrap();
    std::fs::write(root.path().join("Project/a"), "existing").unwrap();
    let token = CancellationToken::new();
    let error = workflow::download(&client, root.path(), &[job("a")], &token, |event| {
        if matches!(event, DownloadEvent::Progress { .. }) {
            token.cancel();
        }
        Ok(())
    })
    .await
    .unwrap_err();
    assert_eq!(cli::exit_status(&error), 130);
    assert!(
        matches!(error, Error::Download { summary, cause } if summary.completed == 0 && summary.published == 0 && summary.failures.is_empty() && matches!(*cause, Error::Cancelled))
    );
    assert_eq!(
        std::fs::read(root.path().join("Project/a")).unwrap(),
        b"existing"
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("Project"))
            .unwrap()
            .count(),
        1
    );
    shutdown.cancel();
    server.await.unwrap().unwrap();
}
