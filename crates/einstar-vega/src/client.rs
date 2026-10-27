use crate::{CancellationToken, Error, RemotePath, Result, Value, check_cancel};
use cbor4ii::core::{dec::Decode, utils::IoReader};
use reqwest::{Method, RequestBuilder};
use serde::Serialize;
use std::{collections::BTreeMap, io::Cursor, time::Duration};
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// Device metadata preserving CBOR numeric, binary, nested, and tagged values.
pub type DeviceInfo = BTreeMap<String, Value>;

/// Project location and its opaque device-provided date string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInfo {
    pub path: RemotePath,
    pub date_time: String,
}

/// Limits for small metadata requests. Downloads have no overall deadline.
#[derive(Clone, Copy, Debug)]
pub struct RequestPolicy {
    pub timeout: Duration,
    pub metadata_limit: usize,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            metadata_limit: 16 * 1024 * 1024,
        }
    }
}

/// Successfully written bytes, reported after each completed output write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
}

/// A reusable read-only client. It never redirects or retries requests.
#[derive(Clone, Debug)]
pub struct Client {
    base: reqwest::Url,
    http: reqwest::Client,
    policy: RequestPolicy,
}

#[derive(Serialize)]
struct PathRequest<'a> {
    path: &'a str,
}
#[derive(Serialize)]
struct ContentRequest<'a> {
    path: &'a str,
    pos: u64,
    size: u64,
}

impl Client {
    /// Constructs a client for a `host:port` or `[IPv6]:port` address.
    pub fn new(address: &str) -> Result<Self> {
        Self::with_policy(address, RequestPolicy::default())
    }

    /// Constructs a client with explicit metadata limits and request deadline.
    pub fn with_policy(address: &str, policy: RequestPolicy) -> Result<Self> {
        let port = address
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| Error::InvalidPath(address.into()))?;
        let base = reqwest::Url::parse(&format!("http://{address}/"))
            .map_err(|_| Error::InvalidPath(address.into()))?;
        if base.host_str().is_none()
            || base.port_or_known_default() != Some(port)
            || !base.username().is_empty()
            || base.password().is_some()
            || base.path() != "/"
            || base.query().is_some()
            || base.fragment().is_some()
            || policy.timeout.is_zero()
            || policy.metadata_limit == 0
        {
            return Err(Error::InvalidPath(address.into()));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()?;
        Ok(Self { base, http, policy })
    }

    fn request(&self, method: Method, endpoint: &'static str) -> RequestBuilder {
        let mut url = self.base.clone();
        url.set_path(endpoint);
        self.http.request(method, url)
    }

    async fn send(
        &self,
        request: RequestBuilder,
        endpoint: &'static str,
        token: &CancellationToken,
    ) -> Result<reqwest::Response> {
        let response = tokio::select! { biased;
            _ = token.cancelled() => return Err(Error::Cancelled),
            response = request.send() => response?,
        };
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::Status {
                endpoint,
                status: response.status().as_u16(),
            });
        }
        Ok(response)
    }

    async fn bytes(
        &self,
        request: RequestBuilder,
        endpoint: &'static str,
        limit: usize,
        token: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut response = self
            .send(request.timeout(self.policy.timeout), endpoint, token)
            .await?;
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! { biased;
                _ = token.cancelled() => return Err(Error::Cancelled),
                chunk = response.chunk() => chunk?,
            };
            let Some(chunk) = chunk else { return Ok(bytes) };
            if chunk.len() > limit.saturating_sub(bytes.len()) {
                return Err(Error::Protocol(format!(
                    "{endpoint} response exceeds {limit} bytes"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
    }

    async fn metadata(
        &self,
        endpoint: &'static str,
        body: Option<Vec<u8>>,
        token: &CancellationToken,
    ) -> Result<Value> {
        let request = match body {
            Some(body) => self
                .request(Method::POST, endpoint)
                .header("Content-Type", "application/cbor")
                .body(body),
            None => self.request(Method::GET, endpoint),
        };
        let bytes = self
            .bytes(request, endpoint, self.policy.metadata_limit, token)
            .await?;
        let mut reader = IoReader::new(Cursor::new(&bytes));
        let value = Value::decode(&mut reader).map_err(|e| Error::Protocol(e.to_string()))?;
        if reader.into_inner().position() != bytes.len() as u64 {
            return Err(Error::Protocol("trailing bytes after CBOR response".into()));
        }
        let mut envelope = text_map(value)?;
        envelope
            .remove("result")
            .ok_or_else(|| Error::Protocol("missing result field".into()))
    }

    /// Verifies the scanner's exact `connect success` response.
    pub async fn connect(&self, token: &CancellationToken) -> Result<()> {
        let body = self
            .bytes(self.request(Method::GET, "/connect"), "/connect", 15, token)
            .await?;
        if body != b"connect success" {
            return Err(Error::Protocol("unexpected connect response".into()));
        }
        Ok(())
    }

    /// Reads all device metadata without coercing binary values to strings.
    pub async fn device_info(&self, token: &CancellationToken) -> Result<DeviceInfo> {
        text_map(self.metadata("/request/deviceInfo", None, token).await?)
    }

    /// Reads projects in deterministic name order.
    pub async fn projects(
        &self,
        token: &CancellationToken,
    ) -> Result<BTreeMap<String, ProjectInfo>> {
        let projects = text_map(self.metadata("/request/projectsInfo", None, token).await?)?;
        projects
            .into_iter()
            .map(|(name, value)| {
                let mut value = text_map(value)?;
                let path = RemotePath::parse(take_text(&mut value, "Path")?)?;
                let date_time = take_text(&mut value, "DateTime")?;
                Ok((name, ProjectInfo { path, date_time }))
            })
            .collect()
    }

    /// Reads the flat file list. The undocumented first result item is opaque.
    pub async fn list_files(
        &self,
        path: &RemotePath,
        token: &CancellationToken,
    ) -> Result<Vec<RemotePath>> {
        let body = encode(&PathRequest {
            path: path.as_str(),
        })?;
        let value = self
            .metadata("/request/listFilePaths", Some(body), token)
            .await?;
        let Value::Array(mut result) = value else {
            return Err(Error::Protocol("list result is not an array".into()));
        };
        if result.len() < 2 {
            return Err(Error::Protocol("list result lacks file paths".into()));
        }
        let Value::Array(paths) = result.swap_remove(1) else {
            return Err(Error::Protocol("file paths are not an array".into()));
        };
        paths
            .into_iter()
            .map(|p| match p {
                Value::Text(path) => RemotePath::parse(path),
                _ => Err(Error::Protocol(
                    "file list contains a non-string path".into(),
                )),
            })
            .collect()
    }

    /// Sends the raw path and reads exactly one big-endian unsigned 64-bit size.
    pub async fn file_size(&self, path: &RemotePath, token: &CancellationToken) -> Result<u64> {
        let body = self
            .bytes(
                self.request(Method::POST, "/file/info")
                    .body(path.as_str().to_owned()),
                "/file/info",
                8,
                token,
            )
            .await?;
        let bytes = body.try_into().map_err(|_| {
            Error::Protocol("file size response must contain exactly eight bytes".into())
        })?;
        Ok(u64::from_be_bytes(bytes))
    }

    /// Streams exactly `size` bytes into an async writer; partial writes are
    /// completed before progress is reported. Cancellation can leave bytes in
    /// the caller's writer; use `OutputDirectory` for atomic local replacement.
    pub async fn download<W, F>(
        &self,
        path: &RemotePath,
        size: u64,
        writer: &mut W,
        token: &CancellationToken,
        mut progress: F,
    ) -> Result<()>
    where
        W: AsyncWrite + Unpin,
        F: FnMut(Progress),
    {
        let body = encode(&ContentRequest {
            path: path.as_str(),
            pos: 0,
            size,
        })?;
        let request = self
            .request(Method::POST, "/file/content")
            .header("Content-Type", "application/cbor")
            .body(body);
        let mut response = self.send(request, "/file/content", token).await?;
        let mut downloaded = 0_u64;
        loop {
            let chunk = tokio::select! { biased;
                _ = token.cancelled() => return Err(Error::Cancelled),
                chunk = response.chunk() => chunk?,
            };
            let Some(chunk) = chunk else { break };
            let received = downloaded.saturating_add(chunk.len() as u64);
            if received > size {
                return Err(Error::Size {
                    expected: size,
                    received,
                });
            }
            tokio::select! { biased;
                _ = token.cancelled() => return Err(Error::Cancelled),
                result = writer.write_all(&chunk) => result?,
            };
            downloaded = received;
            progress(Progress {
                downloaded,
                total: size,
            });
        }
        check_cancel(token)?;
        if downloaded != size {
            return Err(Error::Size {
                expected: size,
                received: downloaded,
            });
        }
        if size == 0 {
            progress(Progress {
                downloaded: 0,
                total: 0,
            });
        }
        Ok(())
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    cbor4ii::serde::to_vec(Vec::new(), value).map_err(|e| Error::Protocol(e.to_string()))
}

fn text_map(value: Value) -> Result<BTreeMap<String, Value>> {
    let Value::Map(entries) = value else {
        return Err(Error::Protocol("expected a CBOR map".into()));
    };
    let mut map = BTreeMap::new();
    for (key, value) in entries {
        let Value::Text(key) = key else {
            return Err(Error::Protocol("expected a string map key".into()));
        };
        if map.insert(key, value).is_some() {
            return Err(Error::Protocol("duplicate map key".into()));
        }
    }
    Ok(map)
}

fn take_text(map: &mut BTreeMap<String, Value>, field: &str) -> Result<String> {
    match map.remove(field) {
        Some(Value::Text(value)) => Ok(value),
        _ => Err(Error::Protocol(format!("missing or non-string {field}"))),
    }
}
