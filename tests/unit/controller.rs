use super::*;
use axum::{
    Router,
    body::Bytes,
    http::StatusCode,
    routing::{get, post},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use serde_json::json;

fn temporary() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

async fn server(failure: &'static str) -> (Client, CancellationToken, JoinHandle<io::Result<()>>) {
    fn cbor(value: serde_json::Value) -> Vec<u8> {
        cbor4ii::serde::to_vec(Vec::new(), &value).unwrap()
    }
    let router = Router::new()
        .route(
            "/connect",
            get(move || async move {
                if failure == "connect" {
                    (StatusCode::BAD_GATEWAY, "failed")
                } else {
                    (StatusCode::OK, "connect success")
                }
            }),
        )
        .route(
            "/request/deviceInfo",
            get(move || async move {
                if failure == "info" {
                    vec![255]
                } else {
                    cbor(json!({"result":{"model":"Vega"}}))
                }
            }),
        )
        .route(
            "/request/projectsInfo",
            get(move || async move {
                if failure == "projects" {
                    vec![255]
                } else {
                    cbor(json!({"result":{
                        "Project":{"Path":"/Project","DateTime":"today"},
                        "Logs":{"Path":"/Logs","DateTime":"today"}
                    }}))
                }
            }),
        )
        .route(
            "/request/listFilePaths",
            post(move |bytes: Bytes| async move {
                let request: serde_json::Value = cbor4ii::serde::from_slice(&bytes).unwrap();
                assert_eq!(
                    request["path"], "/Project",
                    "Logs is deliberately omitted from the browser"
                );
                if failure == "list" {
                    vec![255]
                } else if failure == "tree" {
                    cbor(json!({"result":[0,["/Project/a","/Project/a/b"]]}))
                } else {
                    cbor(json!({"result":[0,["/Project/a"]]}))
                }
            }),
        )
        .route(
            "/file/info",
            post(move || async move {
                if failure == "size" {
                    vec![0]
                } else {
                    4_u64.to_be_bytes().to_vec()
                }
            }),
        )
        .route("/file/content", post(|| async { vec![1, 2, 3, 4] }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::new(&listener.local_addr().unwrap().to_string()).unwrap();
    let token = CancellationToken::new();
    let shutdown = token.clone();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    });
    (client, token, handle)
}

#[tokio::test]
async fn metadata_loader_retains_independent_failures_and_valid_updates() {
    for (failure, message, trees) in [
        ("connect", "Could not connect", 0),
        ("info", "fetching device info", 1),
        ("projects", "fetching projects", 0),
        ("list", "listing Project", 0),
        ("tree", "building Project", 0),
        ("size", "getting size", 1),
        ("", "Connected successfully", 1),
    ] {
        let (client, token, server) = server(failure).await;
        let (tx, mut rx) = mpsc::channel(128);
        load(client, CancellationToken::new(), tx).await;
        let mut model = Model::new();
        while let Some(update) = rx.recv().await {
            model.apply(update);
        }
        assert!(
            model.logs.iter().any(|log| log.contains(message)),
            "{failure}: {:?}",
            model.logs
        );
        assert_eq!(model.trees.len(), trees);
        if failure.is_empty() {
            assert_eq!(
                model.info.get("model"),
                Some(&einstar_vega::Value::Text("Vega".into()))
            );
            assert_eq!(
                model.trees["Project"]
                    .node(model.trees["Project"].root())
                    .unwrap()
                    .size,
                Some(4)
            );
        }
        token.cancel();
        server.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn bounded_updates_finish_when_consumer_closes_or_operation_cancels() {
    let (tx, rx) = mpsc::channel(1);
    let token = CancellationToken::new();
    assert!(send(&tx, &token, Update::Log("first".into())).await);
    token.cancel();
    assert!(!send(&tx, &token, Update::Log("full".into())).await);
    drop(rx);
    assert!(!send(&tx, &CancellationToken::new(), Update::Log("closed".into())).await);
    let (client, shutdown, server) = server("").await;
    load(client.clone(), token, tx).await;
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    load(client, CancellationToken::new(), tx).await;
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn native_loop_downloads_selected_directory_then_joins_before_quit() {
    let (client, shutdown, server) = server("").await;
    let output = temporary();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let start = std::time::Instant::now();
    let mut selected = false;
    tokio::time::timeout(
        Duration::from_secs(3),
        run_loop(
            &mut terminal,
            client,
            output.path().into(),
            &CancellationToken::new(),
            || {
                if output.path().join("Project/a").exists() {
                    return Ok(Some(key(KeyCode::Char('q'))));
                }
                if !selected && start.elapsed() > Duration::from_millis(150) {
                    selected = true;
                    return Ok(Some(key(KeyCode::Char('d'))));
                }
                Ok(None)
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        std::fs::read(output.path().join("Project/a")).unwrap(),
        [1, 2, 3, 4]
    );
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Project"));
    assert!(text.contains("Vega"));
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn loop_event_failure_and_parent_cancellation_observe_worker_shutdown() {
    let (client, shutdown, server) = server("").await;
    let output = temporary();
    let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
    let error = run_loop(
        &mut terminal,
        client.clone(),
        output.path().into(),
        &CancellationToken::new(),
        || Err(io::ErrorKind::BrokenPipe.into()),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, Error::Io(ref error) if error.kind() == io::ErrorKind::BrokenPipe));
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        run_loop(&mut terminal, client, output.path().into(), &token, || Ok(
            None
        ))
        .await,
        Err(Error::Cancelled)
    ));
    shutdown.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_observes_download_result_and_both_panicking_joins() {
    for result in [
        Ok(workflow::Summary::default()),
        Err(Error::Cancelled),
        Err(Error::Download {
            summary: workflow::Summary::default(),
            cause: Box::new(Error::Cancelled),
        }),
    ] {
        Workers {
            loader: None,
            download: Some(tokio::spawn(async { result })),
        }
        .shutdown()
        .await
        .unwrap();
    }
    let error = Workers {
        loader: None,
        download: Some(tokio::spawn(async {
            Ok(workflow::Summary {
                completed: 1,
                published: 1,
                failures: vec!["cleanup failed".into()],
            })
        })),
    }
    .shutdown()
    .await
    .unwrap_err();
    assert!(matches!(error, Error::Partial { completed: 1, .. }));
    let error = Workers {
        loader: None,
        download: Some(tokio::spawn(async {
            Err(Error::Download {
                summary: workflow::Summary {
                    completed: 1,
                    published: 1,
                    failures: vec!["cleanup failed".into()],
                },
                cause: Box::new(Error::Cancelled),
            })
        })),
    }
    .shutdown()
    .await
    .unwrap_err();
    assert!(matches!(error, Error::Download { .. }));
    let error = Workers {
        loader: Some(tokio::spawn(async { panic!("loader task failed") })),
        download: Some(tokio::spawn(async { panic!("download task failed") })),
    }
    .shutdown()
    .await
    .unwrap_err();
    assert!(
        matches!(error, Error::Shutdown {cause,shutdown} if matches!(*cause,Error::Worker(_)) && matches!(*shutdown,Error::Worker(_)))
    );
}

#[tokio::test]
async fn completed_workers_reap_and_failed_progress_retains_previous_count() {
    let mut model = Model::new();
    let mut workers = Workers {
        loader: Some(tokio::spawn(async {})),
        download: Some(tokio::spawn(async {
            Ok(workflow::Summary {
                completed: 2,
                published: 2,
                failures: vec!["one failed".into()],
            })
        })),
    };
    while !workers.loader.as_ref().unwrap().is_finished()
        || !workers.download.as_ref().unwrap().is_finished()
    {
        tokio::task::yield_now().await;
    }
    workers.reap(&mut model).await.unwrap();
    assert!(workers.loader.is_none());
    assert!(workers.download.is_none());
    assert!(model.logs.iter().any(|log| log.contains("one failed")));
    assert!(model.logs.back().unwrap().contains("downloaded:2 failed:1"));
    finish_download(
        &mut model,
        Err(Error::Download {
            summary: workflow::Summary {
                completed: 2,
                published: 2,
                failures: vec![],
            },
            cause: Box::new(Error::Io(io::ErrorKind::BrokenPipe.into())),
        }),
    );
    assert!(model.logs.back().unwrap().contains("downloaded:2 failed:1"));
    finish_download(&mut model, Err(Error::Invalid("invalid job".into())));
    assert!(model.logs.back().unwrap().contains("downloaded:0 failed:1"));
}

#[tokio::test]
async fn metadata_connection_deadline_stops_a_real_unresponsive_peer() {
    let release = CancellationToken::new();
    let pending = release.clone();
    let router = Router::new().route(
        "/connect",
        get(move || {
            let pending = pending.clone();
            async move {
                pending.cancelled().await;
                "connect success"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::new(&listener.local_addr().unwrap().to_string()).unwrap();
    let shutdown = CancellationToken::new();
    let server_token = shutdown.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_token.cancelled_owned())
            .await
    });
    let (tx, mut rx) = mpsc::channel(128);
    let started = std::time::Instant::now();
    tokio::time::timeout(
        Duration::from_secs(15),
        load(client, CancellationToken::new(), tx),
    )
    .await
    .unwrap();
    assert!(started.elapsed() >= Duration::from_secs(10));
    let mut model = Model::new();
    while let Some(update) = rx.recv().await {
        model.apply(update);
    }
    assert!(
        model
            .logs
            .iter()
            .any(|message| message.contains("Could not connect") && message.contains("deadline")),
        "{:?}",
        model.logs
    );
    assert!(model.trees.is_empty());
    release.cancel();
    shutdown.cancel();
    server.await.unwrap().unwrap();
}
