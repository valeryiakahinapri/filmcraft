//! Browser client for the loopback cloud-workspace PoC.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8766";
const ASSETS_PATH: &str = "/api/v1/assets";
/// Matches the server's per-upload cap, applied to manifest entries here as a sanity bound.
const MAX_ASSET_BYTES: u64 = 8 << 30;
/// Matches the server's manifest cap; JSON responses above it are refused before parsing.
const MAX_MANIFEST_BYTES: usize = 16 << 20;

thread_local! {
    static BASE_URL: RefCell<String> = const { RefCell::new(String::new()) };
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    id: String,
    source: Variant,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<Variant>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Variant {
    name: String,
    size: u64,
    etag: String,
    source_etag: Option<String>,
}

/// Install the cloud workspace methods under `window.filmcraft.cloudWorkspace`.
pub fn install(filmcraft: &js_sys::Object) -> Result<(), JsValue> {
    let api = js_sys::Object::new();
    set_fn(
        &api,
        "configure",
        Closure::<dyn Fn(String) -> Result<JsValue, JsValue>>::new(|base: String| {
            let url = web_sys::Url::new(&base)?;
            if !matches!(url.protocol().as_str(), "http:" | "https:")
                || !url.username().is_empty()
                || !url.password().is_empty()
                || url.pathname() != "/"
                || !url.search().is_empty()
                || !url.hash().is_empty()
            {
                return Err(js_error("cloud base URL must be an HTTP(S) origin without credentials, path, query, or fragment"));
            }
            let origin = url.origin();
            if origin == "null" {
                return Err(js_error("cloud base URL must have a valid origin"));
            }
            if !is_loopback_origin(&origin) {
                log::warn!("cloud workspace origin {origin} is not loopback; the PoC server is loopback-only");
            }
            BASE_URL.with(|value| *value.borrow_mut() = origin.clone());
            js_sys::JSON::parse(&serde_json::to_string(&json!({"baseUrl": origin})).map_err(|e| js_error(&e.to_string()))?)
        })
        .into_js_value(),
    )?;
    set_fn(
        &api,
        "list",
        Closure::<dyn Fn() -> js_sys::Promise>::new(|| {
            future_to_promise(async {
                let assets = list_assets().await?;
                let value = serde_json::to_value(assets).map_err(|error| js_error(&format!("serialize cloud assets: {error}")))?;
                json_to_js(&value)
            })
        })
        .into_js_value(),
    )?;
    set_fn(
        &api,
        "upload",
        Closure::<dyn Fn(JsValue, JsValue) -> js_sys::Promise>::new(|file: JsValue, options: JsValue| {
            future_to_promise(async move {
                let file = file.dyn_into::<web_sys::File>().map_err(|_| js_error("upload expects a browser File"))?;
                let options = js_to_json(&options)?;
                let kind = options.get("kind").and_then(Value::as_str).unwrap_or("source");
                if !matches!(kind, "source" | "proxy") {
                    return Err(js_error("upload kind must be `source` or `proxy`"));
                }
                let id = match options.get("id").and_then(Value::as_str) {
                    Some(id) => id.to_string(),
                    None if kind == "source" => random_id()?,
                    None => return Err(js_error("proxy upload requires the source asset `id`")),
                };
                validate_id(&id)?;
                let source_etag = options.get("sourceEtag").and_then(Value::as_str);
                if kind == "proxy" && source_etag.is_none() {
                    return Err(js_error("proxy upload requires `sourceEtag` from the current source asset"));
                }
                let headers = web_sys::Headers::new()?;
                headers.set("X-Asset-Name", &file.name())?;
                if let Some(etag) = source_etag {
                    headers.set("X-Source-ETag", etag)?;
                }
                let init = web_sys::RequestInit::new();
                init.set_method("PUT");
                init.set_headers(&headers);
                init.set_body(file.as_ref());
                let url = format!("{}{ASSETS_PATH}/{id}/{kind}", base_url());
                let response = fetch(&url, &init).await?;
                let value = response_value(response).await?;
                json_to_js(&value)
            })
        })
        .into_js_value(),
    )?;
    set_fn(
        &api,
        "cacheProxy",
        Closure::<dyn Fn(String) -> js_sys::Promise>::new(|id: String| {
            future_to_promise(async move {
                let (_, file, cached) = cache_proxy(&id).await?;
                json_to_js(&json!({"cached": cached, "name": file.name(), "size": file.size()}))
            })
        })
        .into_js_value(),
    )?;
    set_fn(
        &api,
        "importAsset",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|id: String, options| future_to_promise(async move { import_asset(&id, &options).await }))
            .into_js_value(),
    )?;
    js_sys::Reflect::set(filmcraft, &"cloudWorkspace".into(), &api)?;
    Ok(())
}

async fn list_assets() -> Result<Vec<Asset>, JsValue> {
    let response = fetch(&format!("{}{ASSETS_PATH}", base_url()), &web_sys::RequestInit::new()).await?;
    let value = response_value(response).await?;
    let assets: Vec<Asset> = serde_json::from_value(value).map_err(|error| js_error(&format!("invalid cloud asset list: {error}")))?;
    // The configured origin's manifest is untrusted input: bound it before its etags reach OPFS
    // paths (`proxy_file_name`) or its sizes reach import decisions.
    for asset in &assets {
        validate_asset(asset)?;
    }
    Ok(assets)
}

/// Validate one manifest entry against the same shapes the server enforces.
fn validate_asset(asset: &Asset) -> Result<(), JsValue> {
    validate_id(&asset.id)?;
    if !valid_etag(&asset.source.etag) || asset.source.size > MAX_ASSET_BYTES {
        return Err(js_error("cloud asset list contains invalid source metadata"));
    }
    if let Some(proxy) = &asset.proxy {
        if !valid_etag(&proxy.etag) || proxy.size > MAX_ASSET_BYTES || proxy.source_etag.as_deref().is_some_and(|etag| !valid_etag(etag)) {
            return Err(js_error("cloud asset list contains invalid proxy metadata"));
        }
    }
    Ok(())
}

/// `sha256-<64 ascii hex>`, like the server's manifest validation.
fn valid_etag(etag: &str) -> bool {
    etag.strip_prefix("sha256-").is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

async fn import_asset(id: &str, options: &JsValue) -> Result<JsValue, JsValue> {
    validate_id(id)?;
    let options = js_to_json(options)?;
    let use_proxy = options.get("useProxy").and_then(Value::as_bool).unwrap_or(true);
    let assets = list_assets().await?;
    let asset = assets.into_iter().find(|asset| asset.id == id).ok_or_else(|| js_error("cloud asset not found"))?;
    let source_url = format!("{}{ASSETS_PATH}/{}/source", base_url(), asset.id);
    let source = file_from_response(fetch(&source_url, &web_sys::RequestInit::new()).await?, &asset.source.etag, &asset.source.name).await?;
    let imported = crate::import::import_files(vec![source]).await;
    if imported.get("errors").and_then(Value::as_array).is_some_and(|errors| !errors.is_empty()) {
        let message = imported["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("; ");
        return Err(js_error(&format!("cloud source import failed: {message}")));
    }
    let item = imported
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_u64)
        .ok_or_else(|| js_error("cloud source import returned no media item"))?;
    let source_path = imported.get("paths").and_then(Value::as_array).and_then(|paths| paths.first()).and_then(Value::as_str).unwrap_or_default().to_string();

    let mut proxy_attached = false;
    let mut proxy_path = None;
    if use_proxy && asset.proxy.as_ref().is_some_and(|proxy| proxy.source_etag.as_deref() == Some(&asset.source.etag)) {
        let (_, proxy_file, _) = cache_proxy_for_source(id, Some(&asset.source.etag)).await?;
        let name = proxy_file.name();
        let path = crate::fs::register_blob(&name, proxy_file.into());
        proxy_path = Some(path.clone());
        let attach =
            crate::import::on_ui(move |web, _| web.app.session.execute("media.attachProxies", json!({"item": item, "path": path})).map_err(|e| e.to_string()))
                .await;
        attach.map_err(|error| js_error(&format!("source imported, but attaching its proxy failed: {error}")))?;
        crate::import::on_ui(|web, _| web.app.session.execute("media.toggleProxies", json!({"enabled": true})).map_err(|e| e.to_string()))
            .await
            .map_err(|error| js_error(&format!("proxy attached, but enabling proxies failed: {error}")))?;
        proxy_attached = true;
    }
    json_to_js(&json!({"assetId": id, "item": item, "sourcePath": source_path, "proxyAttached": proxy_attached, "proxyPath": proxy_path}))
}

async fn cache_proxy(id: &str) -> Result<(Asset, web_sys::File, bool), JsValue> {
    cache_proxy_for_source(id, None).await
}

async fn cache_proxy_for_source(id: &str, expected_source_etag: Option<&str>) -> Result<(Asset, web_sys::File, bool), JsValue> {
    validate_id(id)?;
    if !crate::opfs::available() {
        return Err(js_error("OPFS is unavailable; persistent proxy caching requires browser storage"));
    }
    let asset = list_assets().await?.into_iter().find(|asset| asset.id == id).ok_or_else(|| js_error("cloud asset not found"))?;
    if expected_source_etag.is_some_and(|expected| expected != asset.source.etag) {
        return Err(js_error("cloud source changed during import; refresh the workspace and retry"));
    }
    let proxy = asset
        .proxy
        .as_ref()
        .filter(|proxy| proxy.source_etag.as_deref() == Some(&asset.source.etag))
        .ok_or_else(|| js_error("cloud asset has no current proxy"))?;
    let name = proxy_file_name(&asset);
    let cached = crate::opfs::files_in("cloud-proxies").await;
    for (old_name, _) in &cached {
        if old_name.starts_with(&format!("{id}-")) && old_name != &name {
            crate::opfs::remove(&format!("cloud-proxies/{old_name}")).await;
        }
    }
    if let Some((_, file)) = cached.into_iter().find(|(old_name, file)| old_name == &name && file.size() as u64 == proxy.size) {
        return Ok((asset, file, true));
    }

    let url = format!("{}{ASSETS_PATH}/{id}/proxy", base_url());
    let response = fetch(&url, &web_sys::RequestInit::new()).await?;
    let blob = blob_from_response(response, &proxy.etag).await?;
    if blob.size() as u64 != proxy.size {
        return Err(js_error("cloud proxy size did not match its asset metadata"));
    }
    crate::opfs::store_blob(&format!("cloud-proxies/{name}"), &blob).await?;
    let file = crate::opfs::files_in("cloud-proxies")
        .await
        .into_iter()
        .find(|(file_name, _)| file_name == &name)
        .map(|(_, file)| file)
        .ok_or_else(|| js_error("cached cloud proxy was not found in OPFS"))?;
    Ok((asset, file, false))
}

async fn file_from_response(response: web_sys::Response, expected_etag: &str, name: &str) -> Result<web_sys::File, JsValue> {
    let blob = blob_from_response(response, expected_etag).await?;
    web_sys::File::new_with_blob_sequence(&js_sys::Array::of1(&blob), name)
}

async fn blob_from_response(response: web_sys::Response, expected_etag: &str) -> Result<web_sys::Blob, JsValue> {
    check_response(&response).await?;
    let header = response.headers().get("etag")?.ok_or_else(|| js_error("cloud response omitted its version ETag"))?;
    if header.trim_matches('"') != expected_etag {
        return Err(js_error("cloud file changed during download; refresh the workspace and retry"));
    }
    JsFuture::from(response.blob()?).await?.dyn_into()
}

async fn response_value(response: web_sys::Response) -> Result<Value, JsValue> {
    check_response(&response).await?;
    if let Some(length) = response.headers().get("content-length")?.and_then(|value| value.parse::<u64>().ok())
        && length > MAX_MANIFEST_BYTES as u64
    {
        return Err(js_error("cloud response exceeds the 16 MiB manifest limit"));
    }
    let text = JsFuture::from(response.text()?).await?.as_string().ok_or_else(|| js_error("cloud response was not text"))?;
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(js_error("cloud response exceeds the 16 MiB manifest limit"));
    }
    serde_json::from_str(&text).map_err(|error| js_error(&format!("invalid cloud response JSON: {error}")))
}

async fn check_response(response: &web_sys::Response) -> Result<(), JsValue> {
    if response.ok() {
        return Ok(());
    }
    let status = response.status();
    let text = JsFuture::from(response.text()?).await?.as_string().unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text).ok().and_then(|value| value.get("error").and_then(Value::as_str).map(str::to_string)).unwrap_or(text);
    Err(js_error(&format!("cloud service returned HTTP {status}: {message}")))
}

async fn fetch(url: &str, init: &web_sys::RequestInit) -> Result<web_sys::Response, JsValue> {
    let window = web_sys::window().ok_or_else(|| js_error("browser window is unavailable"))?;
    JsFuture::from(window.fetch_with_str_and_init(url, init)).await?.dyn_into()
}

fn base_url() -> String {
    BASE_URL.with(|value| {
        let configured = value.borrow();
        if configured.is_empty() { DEFAULT_BASE_URL.to_string() } else { configured.clone() }
    })
}

fn random_id() -> Result<String, JsValue> {
    let window = web_sys::window().ok_or_else(|| js_error("browser window is unavailable"))?;
    let crypto = window.crypto()?;
    let function = js_sys::Reflect::get(crypto.as_ref(), &"randomUUID".into())?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| js_error("pass an asset id; this browser does not support crypto.randomUUID"))?;
    function.call0(crypto.as_ref())?.as_string().ok_or_else(|| js_error("browser did not return a cloud asset id"))
}

fn validate_id(id: &str) -> Result<(), JsValue> {
    if id.is_empty() || id.len() > 96 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) {
        return Err(js_error("asset id must contain 1–96 ASCII letters, digits, hyphens, or underscores"));
    }
    Ok(())
}

/// The PoC server binds loopback only; a host page can still point the client at any origin,
/// and that origin then controls the workspace manifest and file bytes, so warn when it is
/// not loopback.
fn is_loopback_origin(origin: &str) -> bool {
    let Ok(url) = web_sys::Url::new(origin) else { return false };
    let host = url.hostname();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]") || host.starts_with("127.")
}

fn proxy_file_name(asset: &Asset) -> String {
    let Some(proxy) = &asset.proxy else { return String::new() };
    let source = asset.source.etag.strip_prefix("sha256-").unwrap_or("unknown");
    let proxy_version = proxy.etag.strip_prefix("sha256-").unwrap_or("unknown");
    let extension: String = proxy.name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("media").chars().filter(char::is_ascii_alphanumeric).take(16).collect();
    let extension = if extension.is_empty() { "media".to_string() } else { extension };
    format!("{}-{source}-{proxy_version}.{extension}", asset.id)
}

fn js_to_json(value: &JsValue) -> Result<Value, JsValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(json!({}));
    }
    let encoded = js_sys::JSON::stringify(value)?.as_string().ok_or_else(|| js_error("options must be JSON serializable"))?;
    serde_json::from_str(&encoded).map_err(|error| js_error(&format!("invalid options JSON: {error}")))
}

fn json_to_js(value: &Value) -> Result<JsValue, JsValue> {
    let encoded = serde_json::to_string(value).map_err(|error| js_error(&format!("serialize cloud result: {error}")))?;
    js_sys::JSON::parse(&encoded)
}

fn set_fn(object: &js_sys::Object, name: &str, function: JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(object, &name.into(), &function).map(|_| ())
}

fn js_error(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}
