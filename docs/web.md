# FilmCraft on the web

`apps/filmcraft-web` runs the same engine and egui UI as the desktop app in the browser: the
workspace compiled to `wasm32-unknown-unknown`, started by eframe's web runner on WebGPU only.
The app is a static site: one `.wasm`, its `wasm-bindgen` JavaScript glue, `index.html`,
an `AudioWorklet` script and an icon. Files stay on the user's machine unless the user explicitly
uses the optional cloud-workspace API described below.

## Build and run

| Tool | Version | Install |
|---|---|---|
| Rust target | `wasm32-unknown-unknown` | `rustup target add wasm32-unknown-unknown` |
| wasm-bindgen CLI | exactly the `wasm-bindgen` crate version (0.2.129) | `cargo install wasm-bindgen-cli --version 0.2.129 --locked` |
| wasm-opt (optional) | any recent binaryen | `brew install binaryen`; used by `cargo xtask web` when found |

```sh
cargo xtask web                 # release build → target/web/dist (index.html, filmcraft_web.js, filmcraft_web_bg.wasm, …)
cargo xtask web --dev           # unoptimised build (faster to compile, slow to run)
cargo xtask web --serve 8765    # build, then serve dist on http://127.0.0.1:8765/
```

The release build uses Rust's optimized profile (opt-level 3, thin LTO, one codegen unit). When
Binaryen's `wasm-opt` is installed, `cargo xtask web` applies its `-O2` pass to the bound WASM as
well. For the current codec set the optimized WASM is 39,689,696 bytes (39.69 MB; 14,658,485 bytes
with gzip). This build carries the layout-profile API (`ui.layout.*` / `filmcraft.exportLayout()`),
the rounded dock cards and the web hamburger menu. It uses WebGPU without eframe's WebGL backend
and omits the optional Noto Serif font on the web. Higher `-O3` post-optimization was not faster
in a local H.264 export comparison.
Browsers remain single-threaded in this build, so software exports cannot use the native encoder
thread pool.

With Brotli quality 11 and a 16 MiB window (`lgwin 24`), the same 39,689,696-byte WASM compresses
to 9,389,752 bytes (8.95 MiB) — the smallest measured HTTP transfer for this build, served as
precompressed bytes with `Content-Encoding: br`. Quality 11 with Brotli's smaller default window
(4 MiB) stays measurably larger, so the large window is what reaches the minimum. The exact artifact is
committed at
[`packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br`](../packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br)
(SHA-256 `5ba94a8546362c5ef8d1c708bd9f1c8595a7af870bfeae5c5acbdee14b2f1fd2`) so the precompressed
build is preserved in git; it decompresses byte-for-byte back to the WASM inside
`packaging/web/filmcraft-web-0.20.1.zip` (SHA-256
`4ca52ab7cdb9aa0f485e04268bea5d40c2731d89b60960bbf9b0291763ec9b7c`). Serving notes for hosts are
in [the web packaging README](../packaging/web/README.md).

`--serve` is a tiny localhost-only static server that sends `Cross-Origin-Opener-Policy` /
`Cross-Origin-Embedder-Policy`, so the page is cross-origin isolated like a production deployment
should be. Any static server works (`python3 -m http.server -d target/web/dist 8765`); it must
serve `.wasm` as `application/wasm` for streaming compilation.

The web crate is empty on non-wasm targets, so `cargo test --workspace` and clippy are
unaffected; `cargo xtask wasm` (part of `cargo xtask ci`) checks it, with `filmcraft-ui-egui`, for
`wasm32-unknown-unknown`.

URL flags: `?empty` (no demo project), `?norecover` (don't reopen the auto-saved project),
`?fresh` (also don't restore media kept in OPFS), `?nowebcodecs` (decode with FilmCraft's own
decoders only), `?host=parent` (host mode: a same-origin page embedding the app
hands it files and takes its saves back instead of downloads; `data-host="page"` on the canvas
is the no-iframe form — [hosting](../packaging/web/README.md)).

If the app panics after start-up (or runs out of memory), `index.html` shows a "FilmCraft stopped
working" overlay with the panic message and a Reload button instead of a frozen canvas, and sets
`window.filmcraftLoad.fatal`. Code the web build runs must not call `std::env::temp_dir`,
`std::time::{Instant, SystemTime}::now`, `std::thread::sleep`/`spawn` or `std::process::id`: they
panic on `wasm32-unknown-unknown` (use `filmcraft_engine::temp_dir`, `web_time`, or a `cfg`).

## How the desktop pieces map to the browser

| Desktop | Web |
|---|---|
| frame worker threads | `FrameServer::pump`: queued frame jobs run on the UI thread between egui frames, highest priority first, within a time budget (6 ms while timeline scrubbing, 24 ms while playing, 40 ms otherwise) |
| export on a worker thread | `filmcraft_export::Exporter`: the export advances one frame per step from `Session::pump_jobs`, 30 ms per UI frame; progress shows in the header like on the desktop |
| `std::fs` (`FsServices`) | `fs::WebServices`: a virtual file table. Picked/dropped files are `File` handles (never copied); files the app writes are kept in memory and offered as downloads |
| file dialogs | File System Access `showOpenFilePicker` where available, else `<input type=file>`; drop files anywhere on the page |
| crash-recovery journal | OPFS: `recovery/snapshot.fcproj` every 5 s while there are unsaved changes, plus copies of imported media in `media/`; the next visit reopens the snapshot |
| cpal output | WebAudio `AudioWorklet` |
| VideoToolbox & co. | WebCodecs `VideoDecoder` |
| TCP control channel / MCP | `window.filmcraft` (below) |

### Threads

wasm threads need a cross-origin isolated page **and** a wasm build with atomics (a nightly
`build-std` toolchain), so this build is single-threaded everywhere: frame rendering, decoding,
mixing and encoding are cooperative on the UI thread, and `rayon` runs inline.
`filmcraft.info()` reports `crossOriginIsolated` so a threaded build can choose at start-up.
The H.264 encoder's native Rayon pool is enabled only on native targets; WASM H.264 export runs
inline instead of trying to create an unsupported one-thread pool.
Commands that need a thread on the desktop (render previews, proxies, Project Manager, mask
tracking) report an error on the web for now.

### Media reads

Containers are opened through `Services::reader` (a `filmcraft_media::ByteReader`): MP4/MOV and
Matroska read their index at import and single samples afterwards, so a multi-gigabyte file is
never loaded whole. Browser `Blob` reads are asynchronous, so `BlobReader` keeps a 384 MiB LRU cache
of 1 MiB chunks: a read whose chunks are missing starts fetching them (plus three chunks of
read-ahead) and fails with `WouldBlock`, setting `filmcraft_media::pending`. The frame server then
drops that frame's result and retries the job after the bytes arrive (the fetch wakes the UI);
the stepped exporter re-renders the frame; an import retries `file.import`. Before importing, the
container index is prefetched (an MP4's `moov` box wherever it is, a small Matroska file whole).
Stills, WAV and compressed-audio files are decoded whole, as on the desktop.

Limits: Matroska/WebM files larger than the chunk cache import slowly (opening scans every
cluster header); a file the user picked is only readable while the page is open, unless its OPFS
copy exists (files up to 4 GiB are copied in the background).

### Decoding

FilmCraft's own decoders (H.264, HEVC, VP9, AV1, ProRes, DNxHR, MJPEG, AAC, Opus…) run in the
browser unchanged. When the browser has WebCodecs, H.264, HEVC, VP9 and AV1 video in MP4/MOV is
decoded by the browser's (usually hardware) `VideoDecoder` instead: `webcodecs::reader_opener` is
registered ahead of the built-in openers and opens such files as a `WcSource`, whose audio and
media info come from our `Mp4Source`. WebCodecs is asynchronous, so a frame request feeds samples
to the decoder and returns "pending"; the decoder's output callback converts the `VideoFrame` to
RGBA (drawn on an `OffscreenCanvas`) into a per-source frame cache, and the retried request finds
it. Codec support is probed at start-up with `VideoDecoder.isConfigSupported`; any decoder error
switches that source to our decoder.

### Audio

`audio::Handle` implements `AudioOut`. The UI thread mixes the sequence 250 ms ahead in 2048-frame
blocks and posts them to the worklet (`web/audio-worklet.js`), which counts the frames it really
played and reports them with its `currentTime`. `played_frames` extrapolates from the last report
on the audio context's clock, never past what was queued — so the audio clock is the playback
master as on the desktop, and a starved worklet holds the playhead. Browsers start audio suspended
until the first click or key press; until then playback runs on the wall clock.

### GPU

The browser build contains only eframe's WebGPU backend; WebGL2 and the `?webgl`/`?cpu` launch
overrides are not included. Serve it over HTTPS (or localhost) in a browser with WebGPU enabled.
FilmCraft's `filmcraft-gpu` compositor uses the adapter when it supports the required floating
point render targets; otherwise the app reports that it is using the CPU compositor. Check
`filmcraft.info().backend` and `.compositor`.

The reported size delta is for the combined WebGPU-only dependency selection and removal of the
optional Noto Serif web font; it is not a per-module profile. Noto Serif's source file is 482,540
bytes. The CPU renderer and codec implementations remain because the bridge, exports, and GPU
fallback paths still use them; their savings have not been independently measured and they are not
safe removal candidates without further call-graph and feature analysis.

While dragging the Timeline playhead, the browser requests preview frames no more often than 15 fps
and caps their resolution at one half. It gives each cooperative frame-pump pass a 6 ms budget so
input and painting can proceed; on release it immediately requests the normal paused resolution. Audio
scrubbing is disabled in the browser to avoid mixing audio on the UI thread while dragging. These
caps key on the Timeline scrub drag; the Program monitor's own scrub bar and timecode drags are
covered by the pump budget but not the 15 fps / half-scale caps.
The optional Noto Serif face is not bundled for the web; unknown or unavailable families fall back
to Inter, while native builds retain all bundled fonts.

## `window.filmcraft`: the agent / test API

The control channel of the desktop app (`docs/control-protocol.md`) as promises. Results resolve
with the method's `result`, errors reject.

`request`, `execute`, `commands`, `uiInspect` and `screenshot` come from [`craft-web-api`](https://github.com/storytold/craft-libs/tree/main/crates/web-api) (storytold/craft-libs#413), the same shape in every Craft app; a call the app does not answer within 30 seconds rejects. The rest are FilmCraft's own.

| Call | |
|---|---|
| `filmcraft.execute(command, params)` | run an engine command (same ids and params as MCP / `filmcraft-cli`) |
| `filmcraft.request(method, params)` | any control method: `ui.inspect`, `ui.click`, `ui.key`, `ui.playback`, `ui.menu.invoke`… |
| `filmcraft.commands()` | the command list |
| `filmcraft.inspect()` / `filmcraft.uiInspect(params)` | `ui.inspect` |
| `filmcraft.screenshot(params)` | `{pngBase64, width, height}` of the canvas (or `{panel}`), and the same PNG as `data` |
| `filmcraft.exportLayout()` | portable, versioned JSON object for the visible editor's current layout |
| `filmcraft.applyLayout(layout, options?)` | apply an exported object or JSON string; options may include `{workspace: "Color", save: true}` |
| `filmcraft.importFiles(files)` | import `File` / `FileList` / `File[]`; resolves `{items, errors, paths}` |
| `filmcraft.importUrl(url, name?)` | fetch a URL and import it (tests) |
| `filmcraft.openProject(fileOrUrl)` | open a `.fcproj` |
| `filmcraft.files()` | the virtual files: `{path, size, kind}` |
| `filmcraft.readFile(path)` | a file's bytes (`Uint8Array`), e.g. an export |
| `filmcraft.info()` | backend, compositor, isolation, OPFS, WebCodecs support and counters, start-up time |

`window.filmcraftLoad` holds `{wasmMs, readyMs}` (module fetch+compile, and until the app runs).

`exportLayout()` captures the live UI session, not the independent headless `FilmCraftBridge`.
Save the returned object as JSON with `JSON.stringify(await filmcraft.exportLayout(), null, 2)`.
The versioned `filmcraft.ui-layout` profile includes the entire docking tree (active tabs, adaptive
ratios, fixed point sizes, accordion/minimized state, floating panels), focused panel, remembered
floating-panel locations and layout-related panel state, including Timeline track/header sizing and
view toggles. Floating rectangles are normalized to the viewport and rescaled on apply; desktop-only
native-window placement is excluded. It deliberately does not capture project-specific Timeline
zoom/scroll or selections.

`applyLayout(profile)` accepts that object or its JSON text and applies it to the workspace named
inside the profile. Use `applyLayout(profile, {workspace: "Color"})` to apply it to another existing
workspace. The optional `save: true` also stores the dock tree as that workspace's saved layout;
the profile JSON itself is the portable way to preserve and reapply the additional panel UI state:

```js
const profile = await filmcraft.exportLayout();
const json = JSON.stringify(profile, null, 2); // save or download this JSON
await filmcraft.applyLayout(json, {workspace: "Color", save: true});
```

Applying a preset validates it before changing the visible UI; malformed or over-1-MiB profiles
reject with an error.

```js
await filmcraft.importUrl("/clip.mp4");
await filmcraft.execute("file.newSequence", {fromItem: 48});
await filmcraft.request("ui.playback", {action: "play"});
const {job} = await filmcraft.execute("file.exportMedia", {format: "h264", path: "/exports/out.mp4"});
```

### Headless `FilmCraftBridge`

`filmcraft.createBridge()` creates an independent engine session for scripts and hosts. It does
not change the document open in the visible editor. Its `commands`, `describe`, `execute`, and
JSON-Lines `run` methods use the same engine registry and parameter schemas as `filmcraft-cli`;
all engine command IDs are available. It also provides CLI-equivalent project/sequence inspection,
media probing (including explicitly supplied numbered image sequences), Program-frame rendering,
import, save, export, and bounded decode benchmarking:

```js
const bridge = filmcraft.createBridge();
const wavBytes = new Uint8Array(await file.arrayBuffer());
const imported = await bridge.importFile(file.name, wavBytes);
const item = imported.items[0];
await bridge.execute("file.newSequence", {fromItem: item});
await bridge.execute("timeline.place", {item, time: 0});
const exported = await bridge.exportMedia({format: "wav", path: "review.wav"});
const audio = bridge.readFile(exported.path); // Uint8Array
```

For several files, `importFiles([{name, bytes}, ...])` returns the CLI-style `{items, errors, paths}` result. Each import call accepts up to 1024 files and at most 512 MiB of bytes in total.

`registerFile(name, bytes)` gives callers a bridge-scoped virtual path for other file-based engine
commands; `files()` and `readFile(path)` only expose files belonging to that bridge. `saveProject`
stores a `.fcproj` in that virtual filesystem. `renderFrame(seconds?, scale?)` returns
`{width, height, rgba, offlineMedia}` with raw RGBA8 pixels; `probe(name, bytes)` returns common
media info plus the same MXF, MPEG, Ogg and BWF details as the CLI. `probeImageSequence([{name,
bytes}, ...])` probes a numbered sequence from explicitly supplied files (the first file selects
the frame-number range). `benchmarkDecode(name, bytes, frames?)` benchmarks sequential video decode
(120 frames by default, up to 10000) in batches that yield to the browser. Byte-array inputs are
limited to 512 MiB. `exportMedia` waits for completion by default,
like the CLI; callers who execute `file.exportMedia` with `wait: false` advance the returned job
with `pumpJobs()` and inspect it with `jobs.list`.

The browser cannot expose local path-based CLI behavior, implicit filesystem directory discovery,
filesystem security flags, MCP stdio/TCP servers, or CLI stdout/stderr. These stay native CLI
features; a host can instead drive the web app through its JavaScript API. Web export and rendering
also run on the browser's main thread (with exports advanced in bounded steps); this bridge does
not claim background or worker execution.

The exported class can also be imported directly from `filmcraft_web.js` by hosts that instantiate
the WASM module themselves:

```js
import init, {FilmCraftBridge} from "./filmcraft_web.js";
await init();
const bridge = new FilmCraftBridge();
```

### Optional cloud workspace PoC

The local Rust service is a development proof of concept, not a production SaaS backend. It binds
only to loopback, has no authentication or encryption, and stores uploaded originals and proxies in
its data directory. Do not expose it to a network or use it for sensitive media.

Start it in one terminal:

```sh
cargo run -p filmcraft-cloud -- --listen 127.0.0.1:8766 --data-dir ./filmcraft-cloud-data
```

In the browser, `filmcraft.cloudWorkspace` provides:

| Call | |
|---|---|
| `configure(origin)` | set the HTTP(S) backend origin; defaults to `http://127.0.0.1:8766` |
| `list()` | list assets and their current source/proxy content-hash versions |
| `upload(file, options?)` | upload a source `File`, or a proxy with `{id, kind: "proxy", sourceEtag}` |
| `cacheProxy(id)` | download the current proxy into OPFS, keyed by source and proxy versions |
| `importAsset(id, options?)` | download/import the original into the visible editor; cache, attach and enable its current proxy when available |

Example:

```js
const source = await filmcraft.cloudWorkspace.upload(masterFile);
await filmcraft.cloudWorkspace.upload(proxyFile, {
  id: source.id,
  kind: "proxy",
  sourceEtag: source.source.etag,
});
const imported = await filmcraft.cloudWorkspace.importAsset(source.id);
```

The backend computes SHA-256 ETags while streaming uploads, supports byte-range reads, and rejects a
proxy upload if its `sourceEtag` does not match the current source. Replacing a source invalidates
its proxy. The browser cache filename contains both current versions; on the next cache/import call,
an outdated proxy is removed and fetched again. OPFS is subject to browser quota and eviction.
The original is fetched as a browser `Blob` and FilmCraft reads it in chunks; the optional proxy is
cached locally for playback. This PoC does not transcode proxies: upload a proxy created elsewhere
or by a native FilmCraft workflow. The production replacement will need authenticated access,
tenant/workspace isolation, quotas, HTTPS, and object storage.

## Browser test

The Node bridge oracle compares command metadata, descriptions, and JSON-Lines results against
`filmcraft-cli`, then tests rendering, media import/probe, export bytes, and project round-tripping
through the actual WASM exports:

```sh
cargo build --locked -p filmcraft-cli
cargo build --locked --target wasm32-unknown-unknown -p filmcraft-web
mkdir -p target/web/node
wasm-bindgen --target nodejs --out-dir target/web/node target/wasm32-unknown-unknown/debug/filmcraft_web.wasm
node apps/filmcraft-web/tests/js-bridge.mjs
```

`apps/filmcraft-web/tests/smoke.mjs` drives headless Chrome over the DevTools protocol (Node ≥ 22,
no npm packages): load, demo playback, import of a generated MP4, playback, H.264 export
(download). It writes screenshots and `report.json`:

```sh
cargo xtask web --serve 8765 &
ffmpeg -f lavfi -i testsrc2=size=640x360:rate=30 -f lavfi -i sine -t 3 -c:v libx264 -pix_fmt yuv420p -c:a aac target/web/dist/web-test.mp4
node apps/filmcraft-web/tests/smoke.mjs --url http://127.0.0.1:8765/ --media web-test.mp4 --out target/web/smoke
```
