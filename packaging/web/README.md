# Hosting FilmCraft for the web

`filmcraft-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `filmcraft-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `filmcraft_web.js` | wasm-bindgen glue (generated, ES module) |
| `filmcraft_web_bg.wasm` | The app, 39,126,617 bytes (39.13 MB) in the current optimized build, or 14,516,260 bytes (14.52 MB) with gzip |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## Any path works

All URLs in `index.html` are relative, so the site works at a domain root
(`https://example.com/`), under a prefix (`https://example.com/tools/filmcraft/`) and from a CDN
bucket. The assets carry a content
hash in their URL (`index.html` loads them with `?v=<build hash>`), so they can be cached
forever. Only `index.html` needs revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm`. Browsers refuse to stream-compile it under
  any other type, and the app then loads slowly or not at all. Serve `.js` as `text/javascript`.
  Most hosts already do both. For nginx, check that `mime.types` has `application/wasm wasm;`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`. That takes the
  current WASM download from 39.13 MB to 14.52 MB with gzip. You can also precompress
  (`brotli -k *.wasm`) and let the server send `Content-Encoding: br`.
- **Precompressed Brotli artifact (recommended).** The repository ships the q11-compressed
  WASM at [`filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br`](filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br):
  **9,314,350 bytes** (8.88 MiB), the smallest measured HTTP transfer for this build, produced from
  the WASM in `filmcraft-web-0.20.1.zip` (39,126,617 bytes, SHA-256
  `fd9051456d934eee007282e0676fa15f177d0c0d7c45a6c1c84767d5c452cc7c`) with Brotli quality 11 and a
  16 MiB window (`lgwin 24`; SHA-256 of the `.br` file:
  `8487a26883c3655f0cf561efdbdfd266ffd350d0afb028b29b192bd171a1327a`, round-trip verified). To serve
  it, publish `filmcraft_web_bg.wasm.br` next to `filmcraft_web_bg.wasm`, send
  `Content-Encoding: br` + `Content-Type: application/wasm` for it, and use `brotli`-aware
  negotiation (`Accept-Encoding: br` → the `.br` file; anything else → the raw `.wasm`). Hosts that
  pick up sibling `.br` files automatically (Netlify, Cloudflare Pages with Brotli enabled) need no
  extra rule; nginx can use `brotli_static on`. Apache needs `mod_brotli` or an explicit rewrite for
  `.wasm.br`. Keep the raw `.wasm` published as the fallback for clients that do not send
  `Accept-Encoding: br`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the `.wasm` and `.js`
  files (their URLs carry the build hash), and `no-cache` on `index.html`. `index.html` must not
  be cached, or a browser can pair an old page with new files.
- **HTTPS:** WebGPU (and the clipboard) only work in a secure context, which means `https://`
  or `http://localhost`. The app does not fall back to WebGL2 or CPU rendering when WebGPU is
  unavailable.
- **No special isolation headers:** FilmCraft doesn't use `SharedArrayBuffer`, so it doesn't
  need `Cross-Origin-Opener-Policy` or `Cross-Origin-Embedder-Policy`. If your site already sends
  COEP `require-corp`, also send `Cross-Origin-Resource-Policy: same-origin` (or `cross-origin`
  when the files live on a CDN) on the app's files.

nginx example:

```nginx
location /filmcraft/ {
    types { application/wasm wasm; text/javascript js; text/html html; }
    gzip on;
    gzip_types application/wasm text/javascript text/html;
    location ~* \.(wasm|js)$ { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location ~* index\.html$ { add_header Cache-Control "no-cache"; }
}
```

Local test: `python3 -m http.server 8765` inside the folder, then open http://localhost:8765/.

## Embedding in a page (iframe)

```html
<iframe
  src="https://example.com/filmcraft/"
  title="FilmCraft video editor"
  style="width: 100%; height: 720px; border: 0;"
  allow="fullscreen; clipboard-read; clipboard-write"
  allowfullscreen>
</iframe>
```

- The app fills the iframe and follows its size, so size the iframe and not the app.
- Keyboard shortcuts go to the iframe after the user clicks into it, as with any embedded app.
- **Cross-origin embeds** work. Preferences are kept in the iframe's `localStorage`. Browsers
  that partition or block third-party storage may forget them between visits, and the app
  then starts with defaults.
- **Sandboxed iframes** need at least
  `sandbox="allow-scripts allow-same-origin allow-downloads allow-popups"`. Without
  `allow-same-origin` there's no storage. Without `allow-downloads`, Save and Export (browser
  downloads) are blocked.
- Don't send `X-Frame-Options: DENY` or a `frame-ancestors` CSP that excludes the embedding page.

## The parent bridge (`?host=parent`)

A page framing FilmCraft in a **same-origin** iframe can add `?host=parent` to the iframe's URL
(`https://example.com/filmcraft/?host=parent`) and have FilmCraft hand its files over instead of
downloading them. The two sides talk `postMessage` with plain window messages that carry a `type`:

| Message | Direction | Fields |
|---|---|---|
| `filmcraft:ready` | app → page | `version`, `appVersion`: the app is running and listening; send a file now |
| `filmcraft:open` | page → app | `name`, `bytes` (an `ArrayBuffer` or `Uint8Array`): a `.fcproj` opens like File › Open, anything else imports like a file dropped on the page. Over 512 MiB or malformed: refused with a message in the app, nothing crashes |
| `filmcraft:save` | app → page | `name`, `saveAs`, `bytes` (a transferred `ArrayBuffer`): every file the app would otherwise download — Save, Save As, Save a Copy, video / frame / audio exports, captions, interchange (AAF, OMF, EDL, markers), extracted audio, the schema backup. The page stores it and reports failures itself. `saveAs` says whether the file becomes the document's file — the project the next Save writes: `true` when the app adopts the file and the page didn't already have it as the document (Save As, and the first Save of a project the page never sent); `false` for a Save of the document under its own name and for every copy (Save a Copy, exports, the schema backup beside an upgraded project) |
| `filmcraft:dirty` | app → page | `dirty`: unsaved changes appeared or went away |
| `filmcraft:failed` | app → page | `error`: the app couldn't start (WebGPU unavailable, for example) |

In host mode the app opens no demo project and restores nothing from an earlier visit (no OPFS
snapshot, no restored media), so the framing page should send the file to edit and guard leaving
the page while changes are unsaved. Written files are also kept in the app's virtual file table
(`filmcraft.files()` / `readFile`), but the host page is their destination. Opening `?host=parent`
outside an iframe gives the normal app with downloads.

## Same-document host mode (no iframe)

A host application can also run FilmCraft from its own document: it loads the wasm-bindgen module
itself and gives FilmCraft one of its own canvases, so there is no iframe and no `?host=parent`.
It opts in with `data-host="page"` on that canvas — the exact value `page` — and the canvas keeps
the id the app looks up (`id="filmcraft_canvas"`).

```html
<canvas id="filmcraft_canvas" data-host="page" style="width: 100%; height: 720px; display: block;"></canvas>
```

The page and the app then talk with DOM `CustomEvent`s on the canvas instead of `postMessage`.
The app dispatches the same messages it would post to a parent — `filmcraft:ready`,
`filmcraft:save`, `filmcraft:dirty`, `filmcraft:failed` — with the message fields (without the
`type`) as the event's `detail`, and it opens a file when the page dispatches a `filmcraft:open`
event with `detail: { name, bytes }`. Events do not bubble, so listen on the canvas itself (two
editors on one page can't hear each other).

Add the listeners before `init`: the app announces itself with `filmcraft:ready` while `init` is
still running, and a file sent in reply would otherwise be missed. And unlike the standalone
page, which starts itself, a host calls the glue's exported `start` with the canvas id after
`init`.

```js
import init, { start } from "./filmcraft_web.js"; // the wasm-bindgen glue shipped with the app

const canvas = document.getElementById("filmcraft_canvas");

canvas.addEventListener("filmcraft:ready", async () => {
  // The app is listening; hand it a project now, if you have one.
  const bytes = await projectBytes(); // an ArrayBuffer or Uint8Array
  canvas.dispatchEvent(new CustomEvent("filmcraft:open", {
    detail: { name: "Trailer.fcproj", bytes },
  }));
});

// Every file the app would otherwise download (Save, Save As, exports, …) arrives here.
canvas.addEventListener("filmcraft:save", (event) => {
  const { name, saveAs, bytes } = event.detail; // bytes is an ArrayBuffer
  // Store the file under `name`; `saveAs` says whether it becomes the document's file.
  // Report your own errors.
  storeFile(name, bytes, saveAs);
});
canvas.addEventListener("filmcraft:dirty", (event) => {
  unsaved = event.detail.dirty; // warn before leaving, as you like
});
canvas.addEventListener("filmcraft:failed", (event) => {
  showError(event.detail.error); // the app couldn't start
});

await init({ module_or_path: "./filmcraft_web_bg.wasm?v=<build hash>" });
await start("filmcraft_canvas"); // the canvas id the app renders on
```

- Saves are not downloads in this mode: the `bytes` in `filmcraft:save` hold the saved file, and
  the host decides where the file goes.
- The assets carry their build hash as a `?v=` query (as `index.html` loads them), so they can
  be cached forever.
- FilmCraft has no user names that work could be signed with, so there is no `?author=`.
- An unframed page that uses `?host=parent` keeps the normal app with downloads.
  `data-host="page"` turns this mode on only when `?host=parent` is absent: if `?host=parent` is
  present it takes precedence, so a same-origin framed page uses the parent bridge, while an
  unframed or cross-origin page gets no bridge and keeps normal downloads.

The messages and both transports are the shared contract of [`craft-web-host`](https://github.com/storytold/craft-libs/tree/main/crates/web-host) (storytold/craft-libs#420), the same for every Craft app: its README lists every message, including `saved`, with the compatibility notes. In `filmcraft:ready`, `version` is the contract's (`"1"`) and `appVersion` is this app's release. FilmCraft reads bytes, so a host sends `bytes`, not a `File`.

### Trust and limits

- **Who is trusted.** In page host mode the page and FilmCraft share one document, so any script on the page can drive the app: open files in it and receive what it saves. Use this mode only for a page you control. The iframe mode (`?host=parent`) answers only the parent window, on the page's own origin.
- **Files from the host are untrusted input.** An opened file goes through the same reading as one picked with File ▸ Open or dropped on the window, with the same format limits; before it is copied into the app's memory, its size is checked against the host open limit (512 MiB) and a larger file is refused, and a name is cut to 255 characters.
- **The event and message names are the interface.** `filmcraft:ready`, `filmcraft:open`, `filmcraft:save`, `filmcraft:dirty` and `filmcraft:failed`, and the fields they carry (the same as the `?host=parent` messages), stay stable; changing one is a breaking change.

## WebGPU requirement

The browser build contains only the WebGPU backend; WebGL2 and the `?webgl`/`?cpu` renderer
overrides are not shipped. Use a WebGPU-capable browser over HTTPS (or localhost). When WebGPU is
unavailable, the page shows an actionable startup error instead of retrying another renderer.
