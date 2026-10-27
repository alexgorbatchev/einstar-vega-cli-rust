use einstar_vega::CancellationToken;
use serde_json::{Value, json};
use std::time::Duration;

struct Server {
    base: String,
    client: reqwest::Client,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Server {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let cancel = CancellationToken::new();
        let task = tokio::spawn(einstar_vega_cli::mock::serve(listener, cancel.clone()));
        Self {
            base,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            cancel,
            task,
        }
    }

    async fn post(&self, route: &str, value: Value) -> reqwest::Response {
        self.raw(route, cbor4ii::serde::to_vec(Vec::new(), &value).unwrap())
            .await
    }

    async fn raw(&self, route: &str, bytes: Vec<u8>) -> reqwest::Response {
        self.client
            .post(format!("{}{route}", self.base))
            .body(bytes)
            .send()
            .await
            .unwrap()
    }

    async fn stop(self) {
        self.cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

async fn decoded(response: reqwest::Response) -> Value {
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/cbor");
    cbor4ii::serde::from_slice(&response.bytes().await.unwrap()).unwrap()
}

#[tokio::test]
async fn metadata_and_project_listing_are_source_fixtures_with_project_scoped_files() {
    let server = Server::start().await;
    let response = server
        .client
        .get(format!("{}/connect", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"connect success");
    let response = server
        .client
        .get(format!("{}/request/deviceInfo", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(
        decoded(response).await["result"],
        json!({"firmwareVersion":"1.3.0-22","model":"Vega","ssdPath":"/mnt/sdcard","batteryLevel":85})
    );
    let response = server
        .client
        .get(format!("{}/request/projectsInfo", server.base))
        .send()
        .await
        .unwrap();
    let projects = decoded(response).await;
    assert_eq!(projects["result"].as_object().unwrap().len(), 2);
    for (project, date) in [
        ("Project1", "2023-10-27T10:00:00Z"),
        ("Project2", "2023-10-28T11:00:00Z"),
    ] {
        let root = format!("/mnt/sdcard/{project}");
        assert_eq!(
            projects["result"][project],
            json!({"Path":root,"DateTime":date})
        );
        let files = decoded(
            server
                .post("/request/listFilePaths", json!({"path":root}))
                .await,
        )
        .await;
        assert_eq!(
            files,
            json!({"result":[0,[format!("{root}/mesh.beb"),format!("{root}/preview.png"),format!("{root}/frame0/DepthImg_0.dat")]]})
        );
        for file in files["result"][1].as_array().unwrap() {
            let path = file.as_str().unwrap();
            let info = server.raw("/file/info", path.as_bytes().to_vec()).await;
            assert_eq!(info.status(), 200);
            assert_eq!(
                info.bytes().await.unwrap().as_ref(),
                &1024_u64.to_be_bytes()
            );
            for (pos, size) in [(0, 1024), (7, 13), (1023, 1), (1024, 0)] {
                let content = server
                    .post("/file/content", json!({"path":path,"pos":pos,"size":size}))
                    .await;
                assert_eq!(content.status(), 200);
                assert_eq!(
                    content.bytes().await.unwrap().as_ref(),
                    vec![0; size as usize]
                );
            }
        }
    }
    server.stop().await;
}

#[tokio::test]
async fn unknown_paths_and_invalid_ranges_never_return_synthetic_success() {
    let server = Server::start().await;
    for path in [
        "/mnt/sdcard/Project3",
        "/mnt/sdcard/Project1/",
        "/mnt/sdcard/Project1/../Project2",
        "/outside",
    ] {
        assert_eq!(
            server
                .post("/request/listFilePaths", json!({"path":path}))
                .await
                .status(),
            404
        );
    }
    for path in [
        "/mnt/sdcard/Project1",
        "/mnt/sdcard/Project1/nope",
        "/mnt/sdcard/Project1/../Project2/mesh.beb",
        "/outside",
    ] {
        assert_eq!(
            server
                .raw("/file/info", path.as_bytes().to_vec())
                .await
                .status(),
            404
        );
        assert_eq!(
            server
                .post("/file/content", json!({"path":path,"pos":0,"size":1}))
                .await
                .status(),
            404
        );
    }
    for (pos, size) in [
        (1024_u64, 1_u64),
        (1025, 0),
        (1023, 2),
        (0, u64::MAX),
        (u64::MAX, 2),
    ] {
        assert_eq!(
            server
                .post(
                    "/file/content",
                    json!({"path":"/mnt/sdcard/Project1/mesh.beb","pos":pos,"size":size})
                )
                .await
                .status(),
            416
        );
    }
    assert_eq!(server.raw("/file/info", vec![0xff]).await.status(), 400);
    server.stop().await;
}

#[tokio::test]
async fn malformed_typed_cbor_trailing_bytes_duplicate_fields_and_large_bodies_are_rejected() {
    let server = Server::start().await;
    for value in [
        json!(null),
        json!({}),
        json!({"path":1}),
        json!({"path":"/mnt/sdcard/Project1","extra":true}),
    ] {
        assert_eq!(
            server.post("/request/listFilePaths", value).await.status(),
            400
        );
    }
    for value in [
        json!({"path":"/mnt/sdcard/Project1/mesh.beb","pos":-1,"size":1}),
        json!({"path":"/mnt/sdcard/Project1/mesh.beb","pos":0,"size":1.5}),
        json!({"path":"/mnt/sdcard/Project1/mesh.beb","pos":0}),
        json!({"path":"/mnt/sdcard/Project1/mesh.beb","pos":0,"size":1,"extra":true}),
    ] {
        assert_eq!(server.post("/file/content", value).await.status(), 400);
    }
    for route in ["/request/listFilePaths", "/file/content"] {
        assert_eq!(server.raw(route, vec![0xa1]).await.status(), 400);
        let value = if route.ends_with("listFilePaths") {
            json!({"path":"/mnt/sdcard/Project1"})
        } else {
            json!({"path":"/mnt/sdcard/Project1/mesh.beb","pos":0,"size":1})
        };
        let mut bytes = cbor4ii::serde::to_vec(Vec::new(), &value).unwrap();
        bytes.push(0);
        assert_eq!(server.raw(route, bytes).await.status(), 400);
        assert_eq!(server.raw(route, vec![0; 2_097_153]).await.status(), 413);
    }
    // Native CBOR map with a duplicated `path` key, independent of map serializers.
    let mut duplicate = vec![0xa2];
    for _ in 0..2 {
        duplicate.extend(cbor4ii::serde::to_vec(Vec::new(), &"path").unwrap());
        duplicate.extend(cbor4ii::serde::to_vec(Vec::new(), &"/mnt/sdcard/Project1").unwrap());
    }
    assert_eq!(
        server
            .raw("/request/listFilePaths", duplicate)
            .await
            .status(),
        400
    );
    server.stop().await;
}

#[tokio::test]
async fn routing_and_shutdown_are_owned_and_listener_is_released() {
    let server = Server::start().await;
    for route in ["/connect", "/request/deviceInfo", "/request/projectsInfo"] {
        assert_eq!(server.raw(route, Vec::new()).await.status(), 405);
    }
    for route in ["/request/listFilePaths", "/file/info", "/file/content"] {
        assert_eq!(
            server
                .client
                .get(format!("{}{route}", server.base))
                .send()
                .await
                .unwrap()
                .status(),
            405
        );
    }
    assert_eq!(
        server
            .client
            .get(format!("{}/unknown", server.base))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let address = server.base.trim_start_matches("http://").to_owned();
    server.stop().await;
    let listener = tokio::net::TcpListener::bind(&address).await.unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    tokio::time::timeout(
        Duration::from_secs(5),
        einstar_vega_cli::mock::serve(listener, cancel),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::net::TcpListener::bind(&address).await.unwrap();
}

#[tokio::test]
async fn cancellation_finishes_an_active_request_with_an_incomplete_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let server = Server::start().await;
    let address = server.base.trim_start_matches("http://");
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(b"POST /file/content HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\nExpect: 100-continue\r\n\r\n").await.unwrap();
    let mut accepted = [0_u8; 25];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut accepted))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&accepted, b"HTTP/1.1 100 Continue\r\n\r\n");
    server.cancel.cancel();
    let mut task = server.task;
    let stopped = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
    if stopped.is_err() {
        // Release the real client so even a failing implementation can drain
        // and its owned server task is observed before the assertion fails.
        drop(socket);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        panic!("cancellation left the server waiting for a request body");
    }
    stopped.unwrap().unwrap().unwrap();
}
