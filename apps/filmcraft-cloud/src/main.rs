#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::error::Error;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::extract::{Path as RoutePath, Request, State};
use axum::http::header::{
    ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, CONTENT_DISPOSITION,
    CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_NONE_MATCH, ORIGIN, RANGE, VARY,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::sync::Mutex;
use tokio_util::io::ReaderStream;
use url::Url;

const MAX_UPLOAD_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_ASSETS: usize = 10_000;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MANIFEST: &str = "assets.json";
const MANIFEST_TMP: &str = "assets.json.tmp";
const MANIFEST_BACKUP: &str = "assets.json.bak";

static NEXT_UPLOAD_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct AppState {
    root: Arc<PathBuf>,
    assets: Arc<Mutex<BTreeMap<String, AssetRecord>>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetRecord {
    id: String,
    source: VariantRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<VariantRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct VariantRecord {
    name: String,
    size: u64,
    etag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_etag: Option<String>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!("filmcraft-cloud: {error}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "cloud storage operation failed")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({"error": self.message}))).into_response()
    }
}

impl AppState {
    async fn open(root: PathBuf) -> Result<Self, Box<dyn Error>> {
        tokio::fs::create_dir_all(&root).await?;
        let manifest = root.join(MANIFEST);
        let backup = root.join(MANIFEST_BACKUP);
        if !tokio::fs::try_exists(&manifest).await? && tokio::fs::try_exists(&backup).await? {
            tokio::fs::rename(&backup, &manifest).await?;
        }
        if tokio::fs::try_exists(&backup).await? {
            tokio::fs::remove_file(&backup).await?;
        }
        if tokio::fs::try_exists(&root.join(MANIFEST_TMP)).await? {
            tokio::fs::remove_file(root.join(MANIFEST_TMP)).await?;
        }
        let assets = if tokio::fs::try_exists(&manifest).await? {
            let file = tokio::fs::File::open(&manifest).await?;
            let mut bytes = Vec::new();
            file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes).await?;
            if bytes.len() > MAX_MANIFEST_BYTES as usize {
                return Err("assets manifest exceeds the 16 MiB limit".into());
            }
            serde_json::from_slice::<BTreeMap<String, AssetRecord>>(&bytes)?
        } else {
            BTreeMap::new()
        };
        if assets.len() > MAX_ASSETS {
            return Err("assets manifest exceeds the 10000 asset limit".into());
        }
        for (id, asset) in &assets {
            if id != &asset.id || !valid_id(id) {
                return Err("assets manifest contains an invalid asset ID".into());
            }
            validate_variant(&asset.source, false)?;
            if !tokio::fs::try_exists(variant_path(&root, id, "source", &asset.source.etag)).await? {
                return Err(format!("source file for asset {id} is missing").into());
            }
            if let Some(proxy) = &asset.proxy {
                validate_variant(proxy, true)?;
                if proxy.source_etag.as_deref() != Some(&asset.source.etag) {
                    return Err(format!("proxy for asset {id} has a stale source version").into());
                }
                if !tokio::fs::try_exists(variant_path(&root, id, "proxy", &proxy.etag)).await? {
                    return Err(format!("proxy file for asset {id} is missing").into());
                }
            }
        }
        Ok(Self { root: Arc::new(root), assets: Arc::new(Mutex::new(assets)) })
    }

    async fn upload(
        &self,
        id: String,
        kind: VariantKind,
        name: String,
        source_etag: Option<String>,
        body: Body,
        headers: &HeaderMap,
    ) -> Result<AssetRecord, ApiError> {
        if !valid_id(&id) {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "asset ID must contain 1–96 ASCII letters, digits, hyphens, or underscores"));
        }
        if let Some(size) = headers.get(CONTENT_LENGTH).and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u64>().ok())
            && size > MAX_UPLOAD_BYTES
        {
            return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "uploads are limited to 8 GiB"));
        }
        let name = safe_asset_name(&name).ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "asset name is invalid"))?;
        let temp_path = upload_temp_path(&self.root);
        let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&temp_path).await.map_err(ApiError::internal)?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let upload = async {
            let mut chunks = body.into_data_stream();
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "upload body could not be read"))?;
                size = size.checked_add(chunk.len() as u64).ok_or_else(|| ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "upload is too large"))?;
                if size > MAX_UPLOAD_BYTES {
                    return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "uploads are limited to 8 GiB"));
                }
                hasher.update(&chunk);
                file.write_all(&chunk).await.map_err(ApiError::internal)?;
            }
            file.sync_all().await.map_err(ApiError::internal)?;
            Ok::<(), ApiError>(())
        }
        .await;
        drop(file);
        if let Err(error) = upload {
            if let Err(cleanup) = tokio::fs::remove_file(&temp_path).await {
                eprintln!("filmcraft-cloud: removing incomplete upload: {cleanup}");
            }
            return Err(error);
        }
        let etag = format!("sha256-{}", hex_digest(&hasher.finalize()));
        let target_path = variant_path(&self.root, &id, kind.as_str(), &etag);
        let mut assets = self.assets.lock().await;
        if kind == VariantKind::Source && !assets.contains_key(&id) && assets.len() >= MAX_ASSETS {
            remove_upload_temp(&temp_path).await;
            return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "workspace is limited to 10000 assets"));
        }
        if let VariantKind::Proxy = kind {
            let Some(current) = assets.get(&id) else {
                remove_upload_temp(&temp_path).await;
                return Err(ApiError::new(StatusCode::NOT_FOUND, "source asset not found"));
            };
            if source_etag.as_deref() != Some(&current.source.etag) {
                remove_upload_temp(&temp_path).await;
                return Err(ApiError::new(StatusCode::CONFLICT, "proxy source version is stale"));
            }
        }
        match tokio::fs::try_exists(&target_path).await {
            Ok(false) => {
                if let Err(error) = tokio::fs::rename(&temp_path, &target_path).await {
                    remove_upload_temp(&temp_path).await;
                    return Err(ApiError::internal(error));
                }
            }
            Ok(true) => {
                if let Err(error) = tokio::fs::remove_file(&temp_path).await {
                    return Err(ApiError::internal(error));
                }
            }
            Err(error) => {
                remove_upload_temp(&temp_path).await;
                return Err(ApiError::internal(error));
            }
        }

        let variant = VariantRecord { name, size, etag, source_etag: (kind == VariantKind::Proxy).then_some(source_etag).flatten() };
        let mut next = assets.clone();
        match kind {
            VariantKind::Source => {
                next.insert(id.clone(), AssetRecord { id: id.clone(), source: variant, proxy: None });
            }
            VariantKind::Proxy => {
                let Some(asset) = next.get_mut(&id) else {
                    return Err(ApiError::new(StatusCode::NOT_FOUND, "source asset not found"));
                };
                asset.proxy = Some(variant);
            }
        }
        write_manifest(&self.root, &next).await.map_err(ApiError::internal)?;
        let Some(record) = next.get(&id).cloned() else {
            return Err(ApiError::internal("uploaded asset was not committed"));
        };
        *assets = next;
        Ok(record)
    }

    async fn variant(&self, id: &str, kind: VariantKind) -> Result<(VariantRecord, PathBuf), ApiError> {
        let assets = self.assets.lock().await;
        let asset = assets.get(id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "asset not found"))?;
        let variant = match kind {
            VariantKind::Source => asset.source.clone(),
            VariantKind::Proxy => {
                let proxy = asset.proxy.as_ref().ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "asset has no current proxy"))?;
                if proxy.source_etag.as_deref() != Some(&asset.source.etag) {
                    return Err(ApiError::new(StatusCode::NOT_FOUND, "asset has no current proxy"));
                }
                proxy.clone()
            }
        };
        let path = variant_path(&self.root, id, kind.as_str(), &variant.etag);
        Ok((variant, path))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VariantKind {
    Source,
    Proxy,
}

impl VariantKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Proxy => "proxy",
        }
    }
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/assets", get(list_assets))
        .route("/api/v1/assets/{id}/source", put(upload_source).get(download_source))
        .route("/api/v1/assets/{id}/proxy", put(upload_proxy).get(download_proxy))
        .layer(middleware::from_fn(cors))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true, "service": "filmcraft-cloud", "apiVersion": 1}))
}

async fn list_assets(State(state): State<AppState>) -> Json<Vec<AssetRecord>> {
    let assets = state.assets.lock().await;
    Json(assets.values().cloned().collect())
}

async fn upload_source(State(state): State<AppState>, RoutePath(id): RoutePath<String>, headers: HeaderMap, body: Body) -> Result<Json<AssetRecord>, ApiError> {
    let name = header_name(&headers)?;
    state.upload(id, VariantKind::Source, name, None, body, &headers).await.map(Json)
}

async fn upload_proxy(State(state): State<AppState>, RoutePath(id): RoutePath<String>, headers: HeaderMap, body: Body) -> Result<Json<AssetRecord>, ApiError> {
    let name = header_name(&headers)?;
    let source_etag = headers.get("x-source-etag").and_then(|value| value.to_str().ok()).map(str::to_string);
    if source_etag.is_none() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "proxy upload requires X-Source-ETag"));
    }
    state.upload(id, VariantKind::Proxy, name, source_etag, body, &headers).await.map(Json)
}

async fn download_source(State(state): State<AppState>, RoutePath(id): RoutePath<String>, headers: HeaderMap) -> Result<Response, ApiError> {
    download_variant(&state, &id, VariantKind::Source, &headers).await
}

async fn download_proxy(State(state): State<AppState>, RoutePath(id): RoutePath<String>, headers: HeaderMap) -> Result<Response, ApiError> {
    download_variant(&state, &id, VariantKind::Proxy, &headers).await
}

async fn download_variant(state: &AppState, id: &str, kind: VariantKind, headers: &HeaderMap) -> Result<Response, ApiError> {
    if !valid_id(id) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid asset ID"));
    }
    let (variant, path) = state.variant(id, kind).await?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(ETAG, etag_header(&variant.etag)?);
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(CONTENT_TYPE, HeaderValue::from_static(mime_type(&variant.name)));
    if let Some(value) = headers.get(IF_NONE_MATCH).and_then(|value| value.to_str().ok())
        && value.split(',').map(str::trim).any(|tag| tag == "*" || tag == format!("\"{}\"", variant.etag))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        *response.headers_mut() = response_headers;
        return Ok(response);
    }

    let range = match headers.get(RANGE).and_then(|value| value.to_str().ok()) {
        Some(value) => {
            Some(parse_range(value, variant.size).ok_or_else(|| ApiError::new(StatusCode::RANGE_NOT_SATISFIABLE, "requested byte range is invalid"))?)
        }
        None => None,
    };
    let (start, length, status) = match range {
        Some((start, end)) => (start, end - start + 1, StatusCode::PARTIAL_CONTENT),
        None => (0, variant.size, StatusCode::OK),
    };
    let mut file = tokio::fs::File::open(path).await.map_err(ApiError::internal)?;
    if start > 0 {
        file.seek(SeekFrom::Start(start)).await.map_err(ApiError::internal)?;
    }
    let stream = ReaderStream::new(file.take(length));
    let body = Body::from_stream(stream);
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response.headers_mut().extend(response_headers);
    response.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from_str(&length.to_string()).map_err(ApiError::internal)?);
    if let Some((start, end)) = range {
        response.headers_mut().insert(CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {start}-{end}/{}", variant.size)).map_err(ApiError::internal)?);
    }
    let filename = safe_header_filename(&variant.name);
    response.headers_mut().insert(CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")).map_err(ApiError::internal)?);
    Ok(response)
}

async fn cors(request: Request, next: Next) -> Response {
    let origin = request.headers().get(ORIGIN).and_then(|value| value.to_str().ok()).filter(|value| is_loopback_origin(value)).map(str::to_string);
    let is_preflight = request.method() == Method::OPTIONS;
    let mut response = if is_preflight { StatusCode::NO_CONTENT.into_response() } else { next.run(request).await };
    if let Some(origin) = origin
        && let Ok(origin) = HeaderValue::from_str(&origin)
    {
        let headers = response.headers_mut();
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(VARY, HeaderValue::from_static("Origin"));
        headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("ETag, Content-Range, Accept-Ranges, Content-Length"));
        if is_preflight {
            headers.insert(ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, PUT, OPTIONS"));
            headers.insert(ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Content-Type, X-Asset-Name, X-Source-ETag, If-None-Match, Range"));
            headers.insert("access-control-max-age", HeaderValue::from_static("600"));
        }
    }
    response
}

fn is_loopback_origin(origin: &str) -> bool {
    let Ok(url) = Url::parse(origin) else { return false };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

fn header_name(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("x-asset-name")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "X-Asset-Name header is required"))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 96 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn safe_asset_name(name: &str) -> Option<String> {
    if name.is_empty() || name.len() > 255 || name == "." || name == ".." {
        return None;
    }
    let clean: String = name.chars().map(|character| if character == '/' || character == '\\' || character.is_control() { '_' } else { character }).collect();
    (!clean.is_empty()).then_some(clean)
}

fn safe_header_filename(name: &str) -> String {
    name.chars().map(|character| if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') { character } else { '_' }).collect()
}

fn validate_variant(variant: &VariantRecord, proxy: bool) -> Result<(), Box<dyn Error>> {
    if safe_asset_name(&variant.name).as_deref() != Some(&variant.name) || !valid_etag(&variant.etag) || variant.size > MAX_UPLOAD_BYTES {
        return Err("assets manifest contains invalid media metadata".into());
    }
    if proxy != variant.source_etag.is_some() || variant.source_etag.as_deref().is_some_and(|etag| !valid_etag(etag)) {
        return Err("assets manifest contains an invalid proxy version".into());
    }
    Ok(())
}

fn valid_etag(etag: &str) -> bool {
    etag.strip_prefix("sha256-").is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn variant_path(root: &Path, id: &str, kind: &str, etag: &str) -> PathBuf {
    let digest = etag.strip_prefix("sha256-").unwrap_or("invalid");
    root.join(format!("{id}-{kind}-{digest}.data"))
}

fn upload_temp_path(root: &Path) -> PathBuf {
    let n = NEXT_UPLOAD_ID.fetch_add(1, Ordering::Relaxed);
    root.join(format!(".upload-{}-{n}.tmp", std::process::id()))
}

async fn remove_upload_temp(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("filmcraft-cloud: removing incomplete upload: {error}");
    }
}

fn hex_digest(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn etag_header(etag: &str) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(&format!("\"{etag}\"")).map_err(ApiError::internal)
}

fn mime_type(name: &str) -> &'static str {
    match Path::new(name).extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') || size == 0 {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(size);
        if suffix == 0 {
            return None;
        }
        return Some((size - suffix, size - 1));
    }
    let start = start.parse::<u64>().ok()?;
    if start >= size {
        return None;
    }
    let end = if end.is_empty() { size - 1 } else { end.parse::<u64>().ok()?.min(size - 1) };
    (end >= start).then_some((start, end))
}

async fn write_manifest(root: &Path, assets: &BTreeMap<String, AssetRecord>) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(assets).map_err(std::io::Error::other)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(std::io::Error::other("assets manifest exceeds the 16 MiB limit"));
    }
    let manifest = root.join(MANIFEST);
    let temp = root.join(MANIFEST_TMP);
    let backup = root.join(MANIFEST_BACKUP);
    let mut file = tokio::fs::File::create(&temp).await?;
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    drop(file);
    let had_manifest = tokio::fs::try_exists(&manifest).await?;
    if had_manifest {
        if tokio::fs::try_exists(&backup).await? {
            tokio::fs::remove_file(&backup).await?;
        }
        tokio::fs::rename(&manifest, &backup).await?;
    }
    if let Err(error) = tokio::fs::rename(&temp, &manifest).await {
        if had_manifest && let Err(restore_error) = tokio::fs::rename(&backup, &manifest).await {
            eprintln!("filmcraft-cloud: restoring assets manifest: {restore_error}");
        }
        return Err(error);
    }
    if had_manifest && let Err(error) = tokio::fs::remove_file(&backup).await {
        eprintln!("filmcraft-cloud: removing old assets manifest: {error}");
    }
    Ok(())
}

fn parse_args() -> Result<(SocketAddr, PathBuf), String> {
    let mut address = "127.0.0.1:8766".parse::<SocketAddr>().map_err(|error| error.to_string())?;
    let mut data_dir = PathBuf::from("filmcraft-cloud-data");
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--listen" => {
                let value = args.next().ok_or("--listen requires an address")?;
                address = value.parse().map_err(|error| format!("invalid --listen address: {error}"))?;
            }
            "--data-dir" => data_dir = PathBuf::from(args.next().ok_or("--data-dir requires a path")?),
            "--help" | "-h" => return Err("usage: filmcraft-cloud [--listen 127.0.0.1:8766] [--data-dir PATH]".into()),
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    if !address.ip().is_loopback() {
        return Err("the PoC backend only binds to a loopback address".into());
    }
    Ok((address, data_dir))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (address, data_dir) = parse_args().map_err(std::io::Error::other)?;
    let state = AppState::open(data_dir.clone()).await?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("FilmCraft cloud PoC: http://{address}/ (data: {})", data_dir.display());
    axum::serve(listener, app(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn request(app: Router, request: Request<Body>) -> Response {
        app.oneshot(request).await.expect("request should reach the router")
    }

    #[tokio::test]
    async fn source_and_proxy_are_versioned_and_ranges_are_served() {
        let directory = tempfile::tempdir().expect("temporary storage");
        let state = AppState::open(directory.path().to_path_buf()).await.expect("open storage");
        let router = app(state.clone());
        let source = b"high-resolution-source";
        let response = request(
            router.clone(),
            Request::builder()
                .method(Method::PUT)
                .uri("/api/v1/assets/asset-1/source")
                .header("x-asset-name", "source.mp4")
                .body(Body::from(source.as_slice()))
                .expect("source upload request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let asset: AssetRecord =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.expect("source upload response")).expect("asset JSON");
        assert_eq!(asset.source.size, source.len() as u64);

        let response = request(
            router.clone(),
            Request::builder().uri("/api/v1/assets/asset-1/source").header(RANGE, "bytes=4-9").body(Body::empty()).expect("range request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers().get(CONTENT_RANGE).and_then(|value| value.to_str().ok()), Some("bytes 4-9/22"));
        assert_eq!(to_bytes(response.into_body(), usize::MAX).await.expect("range body").as_ref(), b"-resol");

        let proxy = b"480p-proxy";
        let response = request(
            router.clone(),
            Request::builder()
                .method(Method::PUT)
                .uri("/api/v1/assets/asset-1/proxy")
                .header("x-asset-name", "proxy.mp4")
                .header("x-source-etag", &asset.source.etag)
                .body(Body::from(proxy.as_slice()))
                .expect("proxy upload request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let with_proxy: AssetRecord =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.expect("proxy upload response")).expect("asset JSON");
        assert!(with_proxy.proxy.is_some());

        let replacement = b"changed-master";
        let response = request(
            router.clone(),
            Request::builder()
                .method(Method::PUT)
                .uri("/api/v1/assets/asset-1/source")
                .header("x-asset-name", "source.mp4")
                .body(Body::from(replacement.as_slice()))
                .expect("replacement upload request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let updated: AssetRecord =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.expect("replacement response")).expect("asset JSON");
        assert_ne!(updated.source.etag, asset.source.etag);
        assert!(updated.proxy.is_none(), "replacing source invalidates its proxy");

        let stale_proxy = request(
            router,
            Request::builder()
                .method(Method::PUT)
                .uri("/api/v1/assets/asset-1/proxy")
                .header("x-asset-name", "stale.mp4")
                .header("x-source-etag", &asset.source.etag)
                .body(Body::from(proxy.as_slice()))
                .expect("stale proxy upload request"),
        )
        .await;
        assert_eq!(stale_proxy.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn loopback_cors_rejects_non_local_origins_and_invalid_ids() {
        let directory = tempfile::tempdir().expect("temporary storage");
        let state = AppState::open(directory.path().to_path_buf()).await.expect("open storage");
        let router = app(state);
        let response = request(
            router.clone(),
            Request::builder().uri("/health").header(ORIGIN, "http://localhost:8765").body(Body::empty()).expect("local origin request"),
        )
        .await;
        assert_eq!(response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).and_then(|value| value.to_str().ok()), Some("http://localhost:8765"));

        let response = request(
            router.clone(),
            Request::builder().uri("/health").header(ORIGIN, "https://example.com").body(Body::empty()).expect("remote origin request"),
        )
        .await;
        assert!(response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none());

        let response = request(
            router,
            Request::builder()
                .method(Method::PUT)
                .uri("/api/v1/assets/../source")
                .header("x-asset-name", "bad.mp4")
                .body(Body::empty())
                .expect("invalid id request"),
        )
        .await;
        assert!(matches!(response.status(), StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND));
    }

    #[test]
    fn byte_ranges_reject_ambiguous_and_out_of_bounds_requests() {
        assert_eq!(parse_range("bytes=2-4", 10), Some((2, 4)));
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=8-", 10), Some((8, 9)));
        assert_eq!(parse_range("bytes=2-1", 10), None);
        assert_eq!(parse_range("bytes=10-", 10), None);
        assert_eq!(parse_range("bytes=1-2,4-5", 10), None);
        assert_eq!(parse_range("bytes=-0", 10), None);
    }
}
