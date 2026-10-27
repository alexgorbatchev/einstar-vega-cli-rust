//! Synthetic offline scanner service. Every fixture is bounded and read-only.

use axum::{
    Router,
    body::Bytes,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cbor4ii::{core::utils::IoReader, serde::Deserializer};
use einstar_vega::CancellationToken;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::io::Cursor;

const PROJECTS: [(&str, &str); 2] = [
    ("/mnt/sdcard/Project1", "2023-10-27T10:00:00Z"),
    ("/mnt/sdcard/Project2", "2023-10-28T11:00:00Z"),
];
const FILES: [&str; 3] = ["mesh.beb", "preview.png", "frame0/DepthImg_0.dat"];
const FILE_SIZE: u64 = 1024;

/// Serves the source's synthetic projects on an already bound listener.
///
/// Cancellation stops accepting connections, cancels incomplete request handling,
/// and awaits connection shutdown. Requests
/// use native HTTP method routing and bounded body extraction. Malformed CBOR
/// returns 400, unknown fixture paths 404, and invalid byte ranges 416.
pub async fn serve(
    listener: tokio::net::TcpListener,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let router = Router::new()
        .route("/connect", get(|| async { "connect success" }))
        .route("/request/deviceInfo", get(device_info))
        .route("/request/projectsInfo", get(projects_info))
        .route("/request/listFilePaths", post(list_files))
        .route("/file/info", post(file_info))
        .route("/file/content", post(file_content))
        .layer(middleware::from_fn_with_state(
            cancel.clone(),
            cancel_request,
        ));
    axum::serve(listener, router)
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
}

async fn cancel_request(
    State(cancel): State<CancellationToken>,
    request: Request,
    next: Next,
) -> Response {
    tokio::select! {
        biased;
        () = cancel.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        response = next.run(request) => response,
    }
}

fn cbor(value: &impl Serialize) -> Result<Response, StatusCode> {
    let bytes =
        cbor4ii::serde::to_vec(Vec::new(), value).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(([(header::CONTENT_TYPE, "application/cbor")], bytes).into_response())
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StatusCode> {
    let reader = IoReader::new(Cursor::new(bytes));
    let mut deserializer = Deserializer::new(reader);
    let value = T::deserialize(&mut deserializer).map_err(|_| StatusCode::BAD_REQUEST)?;
    if deserializer.into_inner().into_inner().position() != bytes.len() as u64 {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value)
}

async fn device_info() -> Result<Response, StatusCode> {
    cbor(&json!({"result": {
        "firmwareVersion": "1.3.0-22",
        "model": "Vega",
        "ssdPath": "/mnt/sdcard",
        "batteryLevel": 85
    }}))
}

async fn projects_info() -> Result<Response, StatusCode> {
    let projects: serde_json::Map<String, serde_json::Value> = PROJECTS
        .iter()
        .map(|(path, date)| {
            let name = path.rsplit('/').next().unwrap_or(path);
            (name.to_owned(), json!({"Path":path,"DateTime":date}))
        })
        .collect();
    cbor(&json!({"result":projects}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathRequest {
    path: String,
}

async fn list_files(bytes: Bytes) -> Result<Response, StatusCode> {
    let request: PathRequest = decode(&bytes)?;
    if !PROJECTS.iter().any(|(path, _)| *path == request.path) {
        return Err(StatusCode::NOT_FOUND);
    }
    let paths: Vec<String> = FILES
        .iter()
        .map(|file| format!("{}/{file}", request.path))
        .collect();
    cbor(&json!({"result":[0,paths]}))
}

fn is_file(path: &str) -> bool {
    PROJECTS.iter().any(|(project, _)| {
        path.strip_prefix(project)
            .and_then(|suffix| suffix.strip_prefix('/'))
            .is_some_and(|file| FILES.contains(&file))
    })
}

async fn file_info(bytes: Bytes) -> Result<Response, StatusCode> {
    let path = std::str::from_utf8(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    if !is_file(path) {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        FILE_SIZE.to_be_bytes().to_vec(),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentRequest {
    path: String,
    pos: u64,
    size: u64,
}

async fn file_content(bytes: Bytes) -> Result<Response, StatusCode> {
    let request: ContentRequest = decode(&bytes)?;
    if !is_file(&request.path) {
        return Err(StatusCode::NOT_FOUND);
    }
    if request.pos > FILE_SIZE || request.size > FILE_SIZE - request.pos {
        return Err(StatusCode::RANGE_NOT_SATISFIABLE);
    }
    // The validated size is at most 1024 on every supported target.
    let size = usize::try_from(request.size).map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?;
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        vec![0_u8; size],
    )
        .into_response())
}
