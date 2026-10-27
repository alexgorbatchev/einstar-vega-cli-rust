use axum::{
    Router,
    body::Bytes,
    http::StatusCode,
    routing::{get, post},
};
use einstar_vega::{CancellationToken, Client, Error, RemotePath};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

async fn server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (address, task)
}

#[test]
fn device_addresses_accept_explicit_default_ports_and_reject_non_authorities() {
    for address in ["127.0.0.1:80", "localhost:80", "[::1]:80", "127.0.0.1:8080"] {
        assert!(Client::new(address).is_ok(), "rejected {address}");
    }
    for address in [
        "localhost",
        "localhost:",
        "localhost:0",
        "localhost:65536",
        "localhost:80/path",
        "user@localhost:80",
        "localhost:80?x",
        "localhost:80#x",
        "::1:80",
    ] {
        assert!(Client::new(address).is_err(), "accepted {address}");
    }
}

#[tokio::test]
async fn wire_requests_use_cbor_structs_raw_path_and_big_endian_size() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let writes = observed.clone();
    #[derive(Deserialize)]
    struct Request {
        path: String,
        pos: u64,
        size: u64,
    }
    let router = Router::new()
        .route("/connect", get(|| async { "connect success" }))
        .route(
            "/file/info",
            post(|body: Bytes| async move {
                assert_eq!(body.as_ref(), b"/projects/Test/mesh.beb");
                5_u64.to_be_bytes().to_vec()
            }),
        )
        .route(
            "/file/content",
            post(move |body: Bytes| {
                let writes = writes.clone();
                async move {
                    let request: Request = cbor4ii::serde::from_slice(&body).unwrap();
                    writes
                        .lock()
                        .unwrap()
                        .push((request.path, request.pos, request.size));
                    b"hello".to_vec()
                }
            }),
        );
    let (address, task) = server(router).await;
    let client = Client::new(&address).unwrap();
    let token = CancellationToken::new();
    client.connect(&token).await.unwrap();
    let path = RemotePath::parse("/projects/Test/mesh.beb").unwrap();
    let size = client.file_size(&path, &token).await.unwrap();
    let mut output = Vec::new();
    let mut progress = Vec::new();
    client
        .download(&path, size, &mut output, &token, |value| {
            progress.push(value)
        })
        .await
        .unwrap();
    assert_eq!(output, b"hello");
    assert_eq!(
        observed.lock().unwrap().as_slice(),
        &[(path.as_str().to_owned(), 0, 5)]
    );
    assert_eq!(progress.last().unwrap().downloaded, 5);
    assert_eq!(progress.last().unwrap().total, 5);
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn unsuccessful_http_status_and_long_size_response_are_rejected() {
    let router = Router::new()
        .route(
            "/connect",
            get(|| async { (StatusCode::BAD_GATEWAY, "connect success") }),
        )
        .route("/file/info", post(|| async { vec![0; 9] }));
    let (address, task) = server(router).await;
    let client = Client::new(&address).unwrap();
    let token = CancellationToken::new();
    assert!(matches!(
        client.connect(&token).await,
        Err(Error::Status { status: 502, .. })
    ));
    assert!(matches!(
        client
            .file_size(&RemotePath::parse("/p/a").unwrap(), &token)
            .await,
        Err(Error::Protocol(_))
    ));
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn truncated_or_oversized_downloads_never_report_success() {
    let (address, task) =
        server(Router::new().route("/file/content", post(|| async { b"abc".to_vec() }))).await;
    let client = Client::new(&address).unwrap();
    let path = RemotePath::parse("/p/a").unwrap();
    for expected in [2, 4] {
        let mut output = Vec::new();
        assert!(
            matches!(client.download(&path, expected, &mut output, &CancellationToken::new(), |_| {}).await, Err(Error::Size { expected: value, .. }) if value == expected)
        );
    }
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn cancellation_interrupts_a_waiting_device_response() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = entered.clone();
    let router = Router::new().route(
        "/connect",
        get(move || {
            let signal = signal.clone();
            async move {
                signal.notify_one();
                std::future::pending::<&str>().await
            }
        }),
    );
    let (address, task) = server(router).await;
    let client = Client::new(&address).unwrap();
    let token = CancellationToken::new();
    let operation = client.connect(&token);
    tokio::pin!(operation);
    tokio::select! {
        _ = entered.notified() => {},
        result = &mut operation => panic!("completed before cancellation: {result:?}"),
    }
    token.cancel();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), operation)
            .await
            .unwrap(),
        Err(Error::Cancelled)
    ));
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn metadata_preserves_native_cbor_values_and_file_listing_request_path() {
    use cbor4ii::core::{Value, enc::Encode, utils::BufWriter};
    fn encode(value: Value) -> Vec<u8> {
        let mut writer = BufWriter::new(Vec::new());
        value.encode(&mut writer).unwrap();
        writer.into_inner()
    }
    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| (Value::Text(k.into()), v))
                .collect(),
        )
    }
    let router = Router::new()
        .route(
            "/request/deviceInfo",
            get(|| async {
                encode(map(vec![(
                    "result",
                    map(vec![
                        ("batteryValue", Value::Float(0.85)),
                        ("serial", Value::Bytes(vec![0, 255])),
                        ("boardInfo", map(vec![("core", Value::Integer(7))])),
                        ("date", Value::Tag(0, Box::new(Value::Text("date".into())))),
                    ]),
                )]))
            }),
        )
        .route(
            "/request/projectsInfo",
            get(|| async {
                encode(map(vec![(
                    "result",
                    map(vec![(
                        "P",
                        map(vec![
                            ("Path", Value::Text("/p/P".into())),
                            ("DateTime", Value::Text("2026-10-14T01:02:03Z".into())),
                        ]),
                    )]),
                )]))
            }),
        )
        .route(
            "/request/listFilePaths",
            post(|body: Bytes| async move {
                let request: std::collections::BTreeMap<String, String> =
                    cbor4ii::serde::from_slice(&body).unwrap();
                assert_eq!(request.get("path").unwrap(), "/p/P");
                // The first result item is opaque; its semantics are undocumented.
                encode(map(vec![(
                    "result",
                    Value::Array(vec![
                        Value::Text("opaque".into()),
                        Value::Array(vec![Value::Text("/p/P/a".into())]),
                    ]),
                )]))
            }),
        );
    let (address, task) = server(router).await;
    let client = Client::new(&address).unwrap();
    let token = CancellationToken::new();
    let info = client.device_info(&token).await.unwrap();
    assert_eq!(info["batteryValue"], Value::Float(0.85));
    assert_eq!(info["serial"], Value::Bytes(vec![0, 255]));
    assert_eq!(
        info["date"],
        Value::Tag(0, Box::new(Value::Text("date".into())))
    );
    let projects = client.projects(&token).await.unwrap();
    assert_eq!(projects["P"].date_time, "2026-10-14T01:02:03Z");
    assert_eq!(
        client
            .list_files(&projects["P"].path, &token)
            .await
            .unwrap(),
        vec![RemotePath::parse("/p/P/a").unwrap()]
    );
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn malformed_metadata_and_nonstring_files_are_errors_instead_of_silent_omissions() {
    let paths = cbor4ii::serde::to_vec(
        Vec::new(),
        &std::collections::BTreeMap::from([("result", (0, vec![42]))]),
    )
    .unwrap();
    let router = Router::new()
        .route("/request/deviceInfo", get(|| async { vec![0xff] }))
        .route("/request/projectsInfo", get(|| async { vec![0xa0] }))
        .route(
            "/request/listFilePaths",
            post(move || {
                let paths = paths.clone();
                async move { paths }
            }),
        );
    let (address, task) = server(router).await;
    let client = Client::new(&address).unwrap();
    let token = CancellationToken::new();
    assert!(matches!(
        client.device_info(&token).await,
        Err(Error::Protocol(_))
    ));
    assert!(matches!(
        client.projects(&token).await,
        Err(Error::Protocol(_))
    ));
    assert!(matches!(
        client
            .list_files(&RemotePath::parse("/p").unwrap(), &token)
            .await,
        Err(Error::Protocol(_))
    ));
    task.abort();
    let _ = task.await;
}

fn encode_value(value: cbor4ii::core::Value) -> Vec<u8> {
    use cbor4ii::core::{enc::Encode, utils::BufWriter};
    let mut writer = BufWriter::new(Vec::new());
    value.encode(&mut writer).unwrap();
    writer.into_inner()
}

#[tokio::test]
async fn malformed_cbor_shapes_duplicates_trailing_data_and_fields_are_rejected() {
    use cbor4ii::core::Value::{self, Array, Integer, Map, Text};
    fn result(value: Value) -> Value {
        Map(vec![(Text("result".into()), value)])
    }
    let malformed_maps = [
        Integer(1),
        Map(vec![(Integer(1), Integer(2))]),
        Map(vec![
            (Text("result".into()), Map(vec![])),
            (Text("result".into()), Map(vec![])),
        ]),
        result(Integer(1)),
    ];
    for value in malformed_maps {
        let bytes = encode_value(value);
        let (address, task) = server(Router::new().route(
            "/request/deviceInfo",
            get(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        ))
        .await;
        assert!(matches!(
            Client::new(&address)
                .unwrap()
                .device_info(&CancellationToken::new())
                .await,
            Err(Error::Protocol(_))
        ));
        task.abort();
        let _ = task.await;
    }
    let mut bytes = encode_value(result(Map(vec![])));
    bytes.push(0);
    let (address, task) = server(Router::new().route(
        "/request/deviceInfo",
        get(move || {
            let bytes = bytes.clone();
            async move { bytes }
        }),
    ))
    .await;
    assert!(matches!(
        Client::new(&address)
            .unwrap()
            .device_info(&CancellationToken::new())
            .await,
        Err(Error::Protocol(_))
    ));
    task.abort();
    let _ = task.await;
    for project in [
        Map(vec![]),
        Map(vec![(Text("Path".into()), Integer(2))]),
        Map(vec![(Text("Path".into()), Text("/p".into()))]),
    ] {
        let bytes = encode_value(result(Map(vec![(Text("P".into()), project)])));
        let (address, task) = server(Router::new().route(
            "/request/projectsInfo",
            get(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        ))
        .await;
        assert!(matches!(
            Client::new(&address)
                .unwrap()
                .projects(&CancellationToken::new())
                .await,
            Err(Error::Protocol(_))
        ));
        task.abort();
        let _ = task.await;
    }
    for list in [
        Integer(1),
        Array(vec![]),
        Array(vec![Integer(0), Integer(1)]),
    ] {
        let bytes = encode_value(result(list));
        let (address, task) = server(Router::new().route(
            "/request/listFilePaths",
            post(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        ))
        .await;
        assert!(matches!(
            Client::new(&address)
                .unwrap()
                .list_files(&RemotePath::parse("/p").unwrap(), &CancellationToken::new())
                .await,
            Err(Error::Protocol(_))
        ));
        task.abort();
        let _ = task.await;
    }
}

#[tokio::test]
async fn metadata_limits_deadlines_redirects_and_exact_handshake_are_enforced() {
    use einstar_vega::RequestPolicy;
    use std::time::Duration;
    for reply in ["wrong", "connect success extra"] {
        let (address, task) =
            server(Router::new().route("/connect", get(move || async move { reply }))).await;
        assert!(matches!(
            Client::new(&address)
                .unwrap()
                .connect(&CancellationToken::new())
                .await,
            Err(Error::Protocol(_))
        ));
        task.abort();
        let _ = task.await;
    }
    let router = Router::new()
        .route(
            "/connect",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                "connect success"
            }),
        )
        .route("/request/deviceInfo", get(|| async { vec![0; 20] }))
        .route("/file/info", post(|| async { vec![0; 7] }))
        .route(
            "/file/content",
            post(|| async {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [("location", "/connect")],
                    "redirect",
                )
            }),
        );
    let (address, task) = server(router).await;
    let client = Client::with_policy(
        &address,
        RequestPolicy {
            timeout: Duration::from_millis(20),
            metadata_limit: 4,
        },
    )
    .unwrap();
    assert!(
        matches!(client.connect(&CancellationToken::new()).await, Err(Error::Http(error)) if error.is_timeout())
    );
    assert!(matches!(
        client.device_info(&CancellationToken::new()).await,
        Err(Error::Protocol(_))
    ));
    assert!(matches!(
        client
            .file_size(
                &RemotePath::parse("/p/a").unwrap(),
                &CancellationToken::new()
            )
            .await,
        Err(Error::Protocol(_))
    ));
    assert!(matches!(
        client
            .download(
                &RemotePath::parse("/p/a").unwrap(),
                0,
                &mut Vec::new(),
                &CancellationToken::new(),
                |_| {}
            )
            .await,
        Err(Error::Status { status: 307, .. })
    ));
    task.abort();
    let _ = task.await;
    for policy in [
        RequestPolicy {
            timeout: Duration::ZERO,
            metadata_limit: 1,
        },
        RequestPolicy {
            timeout: Duration::from_secs(1),
            metadata_limit: 0,
        },
    ] {
        assert!(Client::with_policy("localhost:80", policy).is_err());
    }
}

#[tokio::test]
async fn empty_downloads_report_zero_progress_and_precancelled_calls_do_not_write() {
    let (address, task) =
        server(Router::new().route("/file/content", post(|| async { Vec::<u8>::new() }))).await;
    let client = Client::new(&address).unwrap();
    let path = RemotePath::parse("/p/a").unwrap();
    let mut output = Vec::new();
    let mut progress = Vec::new();
    client
        .download(&path, 0, &mut output, &CancellationToken::new(), |p| {
            progress.push(p)
        })
        .await
        .unwrap();
    assert!(output.is_empty());
    assert_eq!(
        progress,
        vec![einstar_vega::Progress {
            downloaded: 0,
            total: 0
        }]
    );
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        client
            .download(&path, 0, &mut output, &token, |_| panic!(
                "cancelled progress"
            ))
            .await,
        Err(Error::Cancelled)
    ));
    task.abort();
    let _ = task.await;
}

struct PartialWriter {
    bytes: Vec<u8>,
    fail_after: Option<usize>,
    blocked: Option<Arc<tokio::sync::Notify>>,
}

impl tokio::io::AsyncWrite for PartialWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if let Some(signal) = &self.blocked {
            signal.notify_one();
            return std::task::Poll::Pending;
        }
        if self
            .fail_after
            .is_some_and(|limit| self.bytes.len() >= limit)
        {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "test writer full",
            )));
        }
        let amount = bytes.len().min(1);
        self.bytes.extend_from_slice(&bytes[..amount]);
        std::task::Poll::Ready(Ok(amount))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn partial_writes_complete_and_write_errors_do_not_report_unwritten_progress() {
    let (address, task) =
        server(Router::new().route("/file/content", post(|| async { "abc" }))).await;
    let client = Client::new(&address).unwrap();
    let path = RemotePath::parse("/p/a").unwrap();
    let token = CancellationToken::new();
    let mut output = PartialWriter {
        bytes: vec![],
        fail_after: None,
        blocked: None,
    };
    let mut progress = Vec::new();
    client
        .download(&path, 3, &mut output, &token, |p| progress.push(p))
        .await
        .unwrap();
    assert_eq!(output.bytes, b"abc");
    assert_eq!(progress.last().unwrap().downloaded, 3);
    output.bytes.clear();
    output.fail_after = Some(1);
    progress.clear();
    assert!(
        matches!(client.download(&path, 3, &mut output, &token, |p| progress.push(p)).await, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::StorageFull)
    );
    assert_eq!(output.bytes, b"a");
    assert!(progress.is_empty());
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn cancellation_interrupts_a_blocked_output_writer() {
    let (address, task) =
        server(Router::new().route("/file/content", post(|| async { "abc" }))).await;
    let client = Client::new(&address).unwrap();
    let path = RemotePath::parse("/p/a").unwrap();
    let token = CancellationToken::new();
    let entered = Arc::new(tokio::sync::Notify::new());
    let mut output = PartialWriter {
        bytes: vec![],
        fail_after: None,
        blocked: Some(entered.clone()),
    };
    {
        let operation = client.download(&path, 3, &mut output, &token, |_| {
            panic!("blocked write reported progress")
        });
        tokio::pin!(operation);
        tokio::select! {
            _ = entered.notified() => {},
            result = &mut operation => panic!("completed before output blocked: {result:?}"),
        }
        token.cancel();
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), operation)
                .await
                .unwrap(),
            Err(Error::Cancelled)
        ));
    }
    assert!(output.bytes.is_empty());
    task.abort();
    let _ = task.await;
}
