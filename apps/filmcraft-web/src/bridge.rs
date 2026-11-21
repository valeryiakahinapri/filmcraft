//! Headless JavaScript API backed by an independent engine session.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use filmcraft_engine::Session;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

const MAX_FILE_BYTES: usize = 512 << 20;
/// Total bytes one bridge instance may register across its lifetime.
const MAX_TOTAL_FILE_BYTES: usize = 1 << 30;
const MAX_SCRIPT_BYTES: usize = 8 << 20;
const MAX_SCRIPT_LINES: usize = 10_000;
const MAX_IMPORT_FILES: u32 = 1_024;
const MAX_PARAM_BYTES: usize = 16 << 20;

thread_local! {
    static NEXT_BRIDGE_ID: Cell<u64> = const { Cell::new(1) };
}

/// A headless, in-memory FilmCraft session for JavaScript hosts.
///
/// It has its own document and history; it does not operate on the visible editor session. One
/// operation runs at a time: a call made while an earlier call is still pending (its promise has
/// not settled) rejects with an error instead of interleaving, so await each call. The bridge's
/// files live under `/bridge/<id>/`, and `file.open`, `file.import`, `file.saveAs` and
/// `file.exportMedia` accept only paths under that prefix. `probe` parses its bytes
/// synchronously, so keep the input bounded.
#[wasm_bindgen]
pub struct FilmCraftBridge {
    session: RefCell<Session>,
    prefix: String,
    next_file_id: Cell<u64>,
    registered: Cell<usize>,
    busy: Cell<bool>,
}

/// Marks the bridge busy for the duration of one call and releases it on every exit path,
/// so concurrent JavaScript calls reject cleanly instead of fighting over the session.
struct BusyGuard<'a> {
    busy: &'a Cell<bool>,
}

impl<'a> BusyGuard<'a> {
    fn new(busy: &'a Cell<bool>) -> Result<Self, JsValue> {
        if busy.get() {
            return Err(js_error("another bridge operation is still running; await it before calling again"));
        }
        busy.set(true);
        Ok(Self { busy })
    }
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.busy.set(false);
    }
}

#[wasm_bindgen]
impl FilmCraftBridge {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<FilmCraftBridge, JsValue> {
        let id = NEXT_BRIDGE_ID.with(|next| {
            let id = next.get();
            next.set(id.checked_add(1).unwrap_or(0));
            id
        });
        if id == 0 {
            return Err(js_error("bridge instance limit reached"));
        }
        Ok(Self {
            session: RefCell::new(Session::new(Arc::new(crate::fs::WebServices))),
            prefix: format!("/bridge/{id}"),
            next_file_id: Cell::new(1),
            registered: Cell::new(0),
            busy: Cell::new(false),
        })
    }

    /// List commands using the same metadata and case-insensitive ID/label filter as the CLI.
    pub fn commands(&self, filter: Option<String>) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let list = self.session.borrow_mut().execute("command.list", json!({})).map_err(engine_error)?;
        let needle = filter.unwrap_or_default().to_ascii_lowercase();
        let commands = list.as_array().ok_or_else(|| js_error("engine returned an invalid command list"))?;
        let filtered: Vec<Value> = commands
            .iter()
            .filter(|command| {
                needle.is_empty()
                    || command["id"].as_str().unwrap_or_default().to_ascii_lowercase().contains(&needle)
                    || command["label"].as_str().unwrap_or_default().to_ascii_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        to_js(&Value::Array(filtered))
    }

    /// Return the current command metadata for `id`.
    pub fn describe(&self, id: String) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let list = self.session.borrow_mut().execute("command.list", json!({})).map_err(engine_error)?;
        let command = list
            .as_array()
            .and_then(|commands| commands.iter().find(|command| command["id"] == id))
            .ok_or_else(|| js_error(&format!("unknown command `{id}` (try the `commands` method to list ids)")))?;
        to_js(command)
    }

    /// Execute an engine command. Long exports wait for completion unless `params.wait` is false.
    pub async fn execute(&self, id: String, params: JsValue) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let params = from_js(&params)?;
        let result = self.execute_value(&id, params).await?;
        to_js(&result)
    }

    /// Run CLI-style JSON Lines commands (`{"id":"…","params":{…}}` per line).
    ///
    /// Returns per-line results. Invalid lines and command errors are explicit in the result;
    /// execution stops at the first failure unless `keep_going` is true.
    pub async fn run(&self, script: String, keep_going: bool) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        if script.len() > MAX_SCRIPT_BYTES {
            return Err(js_error("script exceeds the 8 MiB limit"));
        }
        if script.lines().count() > MAX_SCRIPT_LINES {
            return Err(js_error("script exceeds the 10000-line limit"));
        }
        let mut results = Vec::new();
        let mut failed = false;
        for (index, line) in script.lines().enumerate() {
            let line_number = index + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let row = match serde_json::from_str::<Value>(trimmed) {
                Ok(value) => {
                    let Some(id) = value.get("id").and_then(Value::as_str) else {
                        failed = true;
                        results.push(json!({"line": line_number, "ok": false, "error": "missing string `id`"}));
                        if !keep_going {
                            break;
                        }
                        continue;
                    };
                    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
                    match self.execute_value(id, params).await {
                        Ok(result) => json!({"line": line_number, "id": id, "ok": true, "result": result}),
                        Err(error) => {
                            failed = true;
                            json!({"line": line_number, "id": id, "ok": false, "error": error.as_string().unwrap_or_else(|| format!("{error:?}"))})
                        }
                    }
                }
                Err(error) => {
                    failed = true;
                    json!({"line": line_number, "ok": false, "error": format!("invalid JSON: {error}")})
                }
            };
            let ok = row["ok"].as_bool() == Some(true);
            results.push(row);
            if !ok && !keep_going {
                break;
            }
        }
        to_js(&json!({"ok": !failed, "results": results}))
    }

    /// Advance background engine jobs. Returns whether jobs remain.
    #[wasm_bindgen(js_name = pumpJobs)]
    pub fn pump_jobs(&self, budget_ms: Option<u32>) -> Result<bool, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let budget_ms = budget_ms.unwrap_or(20);
        if !(1..=100).contains(&budget_ms) {
            return Err(js_error("job budget must be between 1 and 100 milliseconds"));
        }
        Ok(self.session.borrow_mut().pump_jobs(Duration::from_millis(u64::from(budget_ms))))
    }

    /// Inspect the project, active sequence, or both (`all` by default).
    pub fn inspect(&self, target: Option<String>) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let target = target.as_deref().unwrap_or("all");
        let result = match target {
            "project" => self.session.borrow_mut().execute("project.inspect", json!({})),
            "sequence" => self.session.borrow_mut().execute("sequence.inspect", json!({})),
            "all" => {
                let project = self.session.borrow_mut().execute("project.inspect", json!({})).map_err(engine_error)?;
                let sequence = self.session.borrow_mut().execute("sequence.inspect", json!({})).map_err(engine_error)?;
                Ok(json!({"project": project, "sequence": sequence}))
            }
            other => return Err(js_error(&format!("inspect: unknown `{other}` (project | sequence | all)"))),
        };
        to_js(&result.map_err(engine_error)?)
    }

    /// Copy bytes into this bridge's virtual filesystem and return the virtual path.
    #[wasm_bindgen(js_name = registerFile)]
    pub fn register_file(&self, name: String, bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        self.register_file_inner(name, bytes)
    }

    /// Import one byte array into this bridge's project.
    #[wasm_bindgen(js_name = importFile)]
    pub async fn import_file(&self, name: String, bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let path = self.register_file_inner(name, bytes)?;
        let mut result = self.execute_value("file.import", json!({"paths": [path]})).await?;
        result["path"] = json!(path);
        to_js(&result)
    }

    /// Import multiple byte arrays and return the CLI-style `{items, errors, paths}` result.
    #[wasm_bindgen(js_name = importFiles)]
    pub async fn import_files(&self, files: js_sys::Array) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        if files.length() > MAX_IMPORT_FILES {
            return Err(js_error("one import call is limited to 1024 files"));
        }
        let mut inputs = Vec::with_capacity(files.length() as usize);
        let mut total_bytes = 0usize;
        for file in files.iter() {
            let name = js_sys::Reflect::get(&file, &"name".into())?.as_string().ok_or_else(|| js_error("each imported file needs a string `name`"))?;
            let bytes = js_sys::Reflect::get(&file, &"bytes".into())?
                .dyn_into::<js_sys::Uint8Array>()
                .map_err(|_| js_error("each imported file needs Uint8Array `bytes`"))?;
            let size = usize::try_from(bytes.length()).map_err(|_| js_error("file is too large"))?;
            total_bytes = total_bytes
                .checked_add(size)
                .filter(|total| *total <= MAX_FILE_BYTES)
                .ok_or_else(|| js_error("one import call is limited to 512 MiB total"))?;
            inputs.push((name, bytes));
        }

        let mut paths = Vec::new();
        for (name, bytes) in inputs {
            paths.push(self.register_file_inner(name, bytes)?);
        }
        let mut result = self.execute_value("file.import", json!({"paths": paths})).await?;
        result["paths"] = json!(paths);
        to_js(&result)
    }

    /// Open a project from bytes without using a host filesystem path.
    #[wasm_bindgen(js_name = openProject)]
    pub async fn open_project(&self, name: String, bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let path = self.register_file_inner(name, bytes)?;
        let result = self.execute_value("file.open", json!({"path": path})).await?;
        to_js(&result)
    }

    /// Save the bridge's project into its virtual filesystem.
    #[wasm_bindgen(js_name = saveProject)]
    pub async fn save_project(&self, name: String) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let path = self.virtual_path(&name)?;
        let result = self.execute_value("file.saveAs", json!({"path": path})).await?;
        to_js(&json!({"path": path, "result": result}))
    }

    /// Export the active sequence using the engine's full export command and settings schema.
    #[wasm_bindgen(js_name = exportMedia)]
    pub async fn export_media(&self, params: JsValue) -> Result<JsValue, JsValue> {
        let _busy = BusyGuard::new(&self.busy)?;
        let mut params = from_js(&params)?;
        if !params.is_object() {
            return Err(js_error("export parameters must be an object"));
        }
        if let Some(path) = params.get("path").and_then(Value::as_str).map(str::to_owned) {
            if !path.starts_with(&format!("{}/", self.prefix)) {
                params["path"] = json!(self.virtual_path(&path)?);
            }
        }
        let result = self.execute_value("file.exportMedia", params).await?;
        to_js(&result)
    }

    /// Probe an in-memory media file and return the engine's shared media-info schema.
    pub fn probe(&self, name: String, bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
        let size = usize::try_from(bytes.length()).map_err(|_| js_error("file is too large"))?;
        if size > MAX_FILE_BYTES {
            return Err(js_error("in-memory bridge files are limited to 512 MiB"));
        }
        let info = filmcraft_codecs::probe_bytes(&safe_file_name(&name), bytes.to_vec().into()).map_err(|error| js_error(&format!("{name}: {error}")))?;
        to_js(&info)
    }

    /// Probe a numbered still sequence supplied by the browser's file picker.
    #[wasm_bindgen(js_name = probeImageSequence)]
    pub fn probe_image_sequence(&self, files: js_sys::Array) -> Result<JsValue, JsValue> {
        if files.length() == 0 || files.length() > MAX_IMPORT_FILES {
            return Err(js_error("image-sequence probe requires 1 to 1024 files"));
        }
        let mut bytes_by_name = HashMap::with_capacity(files.length() as usize);
        let mut names = Vec::with_capacity(files.length() as usize);
        let mut total_bytes = 0usize;
        for file in files.iter() {
            let name = js_sys::Reflect::get(&file, &"name".into())?.as_string().ok_or_else(|| js_error("each sequence file needs a string `name`"))?;
            let name = safe_file_name(&name);
            if bytes_by_name.contains_key(&name) {
                return Err(js_error("image-sequence file names must be unique"));
            }
            let bytes = js_sys::Reflect::get(&file, &"bytes".into())?
                .dyn_into::<js_sys::Uint8Array>()
                .map_err(|_| js_error("each sequence file needs Uint8Array `bytes`"))?;
            let size = usize::try_from(bytes.length()).map_err(|_| js_error("file is too large"))?;
            total_bytes = total_bytes
                .checked_add(size)
                .filter(|total| *total <= MAX_FILE_BYTES)
                .ok_or_else(|| js_error("image-sequence probe is limited to 512 MiB total"))?;
            names.push(name.clone());
            bytes_by_name.insert(name, bytes.to_vec());
        }
        let first = names.first().ok_or_else(|| js_error("image-sequence probe requires a first file"))?;
        let numbered = filmcraft_media::sequence::Numbered::parse(first).ok_or_else(|| js_error("first sequence file name must contain a frame number"))?;
        let frames = filmcraft_media::sequence::sequence_frames(&numbered, &names);
        let files = Arc::new(bytes_by_name);
        let loader: filmcraft_media::sequence::FrameLoader =
            Arc::new(move |path| files.get(path).cloned().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path.to_string())));
        let source = filmcraft_media::sequence::ImageSequenceSource::new(first, frames, filmcraft_time::FrameRate::FPS_29_97, loader).map_err(engine_error)?;
        let mut info =
            serde_json::to_value(filmcraft_media::MediaSource::info(&source)).map_err(|error| js_error(&format!("serialize media info: {error}")))?;
        let frame_count = u64::try_from(source.frame_count()).map_err(|_| js_error("image sequence has too many frames"))?;
        let last_number = numbered
            .number
            .checked_add(frame_count.checked_sub(1).ok_or_else(|| js_error("image sequence has no frames"))?)
            .ok_or_else(|| js_error("image-sequence frame number overflow"))?;
        let missing: Vec<u64> = source
            .missing_frames()
            .iter()
            .map(|index| {
                u64::try_from(*index)
                    .ok()
                    .and_then(|offset| numbered.number.checked_add(offset))
                    .ok_or_else(|| js_error("image-sequence frame number overflow"))
            })
            .collect::<Result<_, _>>()?;
        info["imageSequence"] = json!({
            "first": source.frame_path(0),
            "firstNumber": numbered.number,
            "lastNumber": last_number,
            "frames": frame_count,
            "missing": missing,
        });
        to_js(&info)
    }

    /// Benchmark sequential video decoding in bounded batches that yield to the browser.
    #[wasm_bindgen(js_name = benchmarkDecode)]
    pub async fn benchmark_decode(&self, name: String, bytes: js_sys::Uint8Array, frames: Option<u32>) -> Result<JsValue, JsValue> {
        let frame_count = frames.unwrap_or(120);
        if !(1..=10_000).contains(&frame_count) {
            return Err(js_error("benchmark frame count must be between 1 and 10000"));
        }
        let size = usize::try_from(bytes.length()).map_err(|_| js_error("file is too large"))?;
        if size > MAX_FILE_BYTES {
            return Err(js_error("in-memory bridge files are limited to 512 MiB"));
        }
        let source = filmcraft_codecs::open_bytes(&safe_file_name(&name), bytes.to_vec().into()).map_err(engine_error)?;
        let rate = source.info().frame_rate();
        let start = web_time::Instant::now();
        for frame in 0..frame_count {
            let frame = i64::from(frame);
            source.video_frame(filmcraft_media::FrameRequest::full(rate.tick_of(frame))).map_err(engine_error)?;
            if frame % 4 == 3 {
                yield_to_event_loop().await?;
            }
        }
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let seconds = elapsed_ms / 1000.0;
        let sequential_fps = if seconds > 0.0 { f64::from(frame_count) / seconds } else { 0.0 };
        let conversion_start = web_time::Instant::now();
        let midpoint = i64::from(frame_count / 2);
        let frame = source.video_frame(filmcraft_media::FrameRequest::full(rate.tick_of(midpoint))).map_err(engine_error)?;
        let (width, height, _) = frame.to_linear_f32_decimated(2);
        let result = json!({
            "frames": frame_count,
            "elapsedMs": elapsed_ms,
            "sequentialFps": sequential_fps,
            "halfResolution": {
                "width": width,
                "height": height,
                "linearConversionMs": conversion_start.elapsed().as_secs_f64() * 1000.0,
            },
        });
        to_js(&result)
    }

    /// Render the active Program frame as raw RGBA8 bytes.
    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&self, seconds: Option<f64>, scale: Option<f32>) -> Result<JsValue, JsValue> {
        if let Some(seconds) = seconds
            && !seconds.is_finite()
        {
            return Err(js_error("seconds must be finite"));
        }
        let scale = scale.unwrap_or(1.0);
        if !scale.is_finite() || scale <= 0.0 {
            return Err(js_error("scale must be finite and positive"));
        }
        let _busy = BusyGuard::new(&self.busy)?;
        let (image, offline) = {
            let mut session = self.session.borrow_mut();
            if let Some(seconds) = seconds {
                session.set_playhead(filmcraft_engine::time::Tick::from_seconds_f64(seconds));
            }
            let playhead = session.playhead();
            session.try_render_program_with_offline_at(scale, playhead).map_err(engine_error)?
        };
        let pixels = js_sys::Uint8Array::from(image.over_black_rgba8().as_slice());
        let result = to_js(&json!({
            "width": image.w,
            "height": image.h,
            "offlineMedia": offline,
        }))?;
        js_sys::Reflect::set(&result, &"rgba".into(), &pixels)?;
        Ok(result)
    }

    /// List virtual files created by this bridge instance.
    pub fn files(&self) -> Result<JsValue, JsValue> {
        let entries = crate::fs::list()
            .into_iter()
            .filter(|(path, _, _)| path.starts_with(&format!("{}/", self.prefix)))
            .map(|(path, size, kind)| json!({"path": path, "size": size, "kind": kind}))
            .collect();
        to_js(&Value::Array(entries))
    }

    /// Read one of this bridge's in-memory files as a `Uint8Array`.
    #[wasm_bindgen(js_name = readFile)]
    pub fn read_file(&self, path: String) -> Result<js_sys::Uint8Array, JsValue> {
        if !path.starts_with(&format!("{}/", self.prefix)) {
            return Err(js_error("path is outside this bridge's virtual filesystem"));
        }
        let bytes = crate::fs::mem_bytes(&path).ok_or_else(|| js_error(&format!("{path}: not found")))?;
        Ok(js_sys::Uint8Array::from(bytes.as_ref()))
    }
}

impl FilmCraftBridge {
    async fn execute_value(&self, id: &str, mut params: Value) -> Result<Value, JsValue> {
        self.check_command_paths(id, &params)?;
        let wait = id == "file.exportMedia" && params.get("wait").and_then(Value::as_bool) != Some(false);
        if wait {
            let Some(object) = params.as_object_mut() else {
                return Err(js_error("command parameters must be an object"));
            };
            object.insert("wait".into(), Value::Bool(false));
        }
        let result = self.session.borrow_mut().execute(id, params).map_err(engine_error)?;
        if !wait {
            return Ok(result);
        }
        let job = result["job"].as_u64().ok_or_else(|| js_error("export did not return a job id"))?;
        loop {
            let finished = {
                let mut session = self.session.borrow_mut();
                session.pump_jobs(Duration::from_millis(20));
                let jobs = session.execute("jobs.list", json!({})).map_err(engine_error)?;
                jobs.as_array()
                    .and_then(|jobs| jobs.iter().find(|entry| entry["id"].as_u64() == Some(job)))
                    .filter(|entry| entry["finished"].as_bool() == Some(true))
                    .cloned()
            };
            if let Some(entry) = finished {
                if let Some(error) = entry["result"]["error"].as_str() {
                    return Err(js_error(&format!("export failed: {error}")));
                }
                let mut completed = result;
                completed["result"] = entry["result"].clone();
                completed["offlineMedia"] = entry["offlineMedia"].clone();
                if let Some(object) = completed.as_object_mut() {
                    object.remove("poll");
                    object.remove("cancel");
                }
                return Ok(completed);
            }
            yield_to_event_loop().await?;
        }
    }

    /// File commands that read or write through the virtual filesystem accept only paths inside
    /// this bridge's prefix, so one bridge cannot reach another bridge's or the visible app's
    /// files.
    fn check_command_paths(&self, id: &str, params: &Value) -> Result<(), JsValue> {
        let prefix = format!("{}/", self.prefix);
        let check = |path: &str| {
            if path.starts_with(&prefix) { Ok(()) } else { Err(js_error("path is outside this bridge's virtual filesystem")) }
        };
        match id {
            "file.open" | "file.saveAs" | "file.exportMedia" => {
                if let Some(path) = params.get("path").and_then(Value::as_str) {
                    check(path)?;
                }
            }
            "file.import" => {
                if let Some(paths) = params.get("paths").and_then(Value::as_array) {
                    for path in paths.iter().filter_map(Value::as_str) {
                        check(path)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn register_file_inner(&self, name: String, bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
        let size = usize::try_from(bytes.length()).map_err(|_| js_error("file is too large"))?;
        if size > MAX_FILE_BYTES {
            return Err(js_error("in-memory bridge files are limited to 512 MiB"));
        }
        let Some(total) = self.registered.get().checked_add(size).filter(|total| *total <= MAX_TOTAL_FILE_BYTES) else {
            return Err(js_error("registered bridge files are limited to 1 GiB in total"));
        };
        self.registered.set(total);
        let id = self.next_file_id.get();
        self.next_file_id.set(id.checked_add(1).ok_or_else(|| js_error("bridge file limit reached"))?);
        let name = safe_file_name(&name);
        let path = format!("{}/{id}-{name}", self.prefix);
        crate::fs::put(&path, bytes.to_vec());
        Ok(path)
    }

    fn virtual_path(&self, name: &str) -> Result<String, JsValue> {
        if name.starts_with(&format!("{}/", self.prefix)) {
            return Ok(name.to_string());
        }
        let name = safe_file_name(name);
        if name.is_empty() {
            return Err(js_error("file name must not be empty"));
        }
        Ok(format!("{}/{}", self.prefix, name))
    }
}

impl Drop for FilmCraftBridge {
    fn drop(&mut self) {
        crate::fs::remove_prefix(&format!("{}/", self.prefix));
    }
}

fn safe_file_name(name: &str) -> String {
    let name: String = name.chars().map(|character| if character == '/' || character == '\\' || character.is_control() { '_' } else { character }).collect();
    if name.is_empty() || name == "." || name == ".." { "file".into() } else { name }
}

fn from_js(value: &JsValue) -> Result<Value, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(json!({}));
    }
    let encoded = js_sys::JSON::stringify(value)?.as_string().ok_or_else(|| js_error("parameters must be JSON serializable"))?;
    if encoded.len() > MAX_PARAM_BYTES {
        return Err(js_error("parameters exceed the 16 MiB limit"));
    }
    serde_json::from_str(&encoded).map_err(|e| js_error(&format!("invalid JSON parameters: {e}")))
}

fn to_js(value: &Value) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(&serde_json::to_string(value).map_err(|e| js_error(&format!("serialize result: {e}")))?)
}

fn engine_error(error: impl std::fmt::Display) -> JsValue {
    js_error(&error.to_string())
}

fn js_error(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}

async fn yield_to_event_loop() -> Result<(), JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let global = js_sys::global();
        let timeout = match js_sys::Reflect::get(&global, &"setTimeout".into()).and_then(JsValue::dyn_into::<js_sys::Function>) {
            Ok(timeout) => timeout,
            Err(error) => {
                let _ = reject.call1(&JsValue::UNDEFINED, &error);
                return;
            }
        };
        let resolve = resolve.clone();
        let callback = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        if let Err(error) = timeout.call2(&global, &callback, &JsValue::from_f64(0.0)) {
            let _ = reject.call1(&JsValue::UNDEFINED, &error);
        }
    });
    JsFuture::from(promise).await.map(|_| ())
}
