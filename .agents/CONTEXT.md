# FilmCraft agent context

This file captures durable project goals and operating context. Current actionable work belongs in
the [handoff](HANDOFF.md), and broad product plans belong in the repository's roadmap documents.

## Product direction and user requirements

FilmCraft is a Rust-native video editor with a browser/WASM build. The user wants the browser app
and JavaScript bridge to be first-class, including cloud-stored media and a realistic hosted demo.
Priorities and constraints stated by the user:

- Keep the JS bridge as close to CLI parity as browser capabilities allow. Expose operations that
  can be exposed; clearly distinguish browser limits rather than silently omitting behavior.
- Build and test both WASM and native Linux headless paths. Do not spend scope on a desktop GUI.
- Keep the dev profile intact while measuring release optimization. Optimize for practical speed
  and also measure size; do not infer performance from artifact size.
- Provide a cloud workspace for originals and proxy media. A proxy may be cached locally for quick
  playback, with cache refresh when source or proxy content changes.
- The longer-term demo goal is an immediately usable browser app, a demo project with video/audio
  and proxy media, and either local browser export or a headless cloud export of the original
  source resolution. The cloud PoC is not itself a production SaaS backend.
- Use rounded-corner styling in FilmCraft's own visual language. Reference projects may inform
  general design concepts, but do not copy their code, artwork, icons, or branding.
- Preserve tested work, report what actually ran, and publish requested artifacts to a feature
  branch; never push directly to `main`.

## Architecture and operating facts

- The browser editor is `apps/filmcraft-web`; it compiles the shared Rust engine to
  `wasm32-unknown-unknown`. `cargo xtask web` builds the static site in `target/web/dist`.
- `window.filmcraft` drives the visible editor. `filmcraft.createBridge()` creates an independent
  headless engine session for script/host use; avoid confusing its project state with the visible
  document.
- Visible-editor layout export/apply also belongs to `window.filmcraft` and the UI control channel,
  not the headless bridge. Its versioned JSON profile preserves the dock tree and layout-related
  panel state, uses viewport-relative floating geometry, and excludes document-specific timeline
  zoom/scroll and native monitor placement.
- The browser filesystem uses virtual files/Blob-backed media and OPFS for recovery/cache. The
  documented cloud API lives in `apps/filmcraft-web/src/cloud.rs`; the Rust PoC service lives in
  `apps/filmcraft-cloud`.
- The cloud PoC is unauthenticated and loopback-only. Do not expose it on a network or treat it as
  secure storage. Production needs authenticated authorization, tenant/workspace isolation, HTTPS,
  quotas, and production object storage. Proxy transcoding and a cloud headless render service are
  separate capabilities; verify their presence before promising them.
- Native builds and web builds have different OS and threading capabilities. Keep native
  parallelism target-specific; the web build is single-threaded unless atomics support is
  deliberately built and deployed.
- WASM artifacts, native build outputs, installed tools, and running servers are ephemeral across
  sandbox changes. Re-bootstrap from checked-in manifests/docs and fetch the branch from GitHub.

## Current published state

The branch is `copilot/wasm-export-performance`, rebased onto `storytold/filmcraft` `main`
(including upstream's slip-monitor and frame-request-view changes) and pushed to
`valeryiakahinapri/filmcraft`. It carries, in order: the earlier bridge/cloud/UI work; the WASM
encoder thread-pool exclusion (native keeps it via target-specific features); the WASM
workflow/handoff docs; the cumulative UI/layout/menu/scrub/theme changes; the committed
Brotli q11 precompressed artifact; a round of review fixes over everything above (bridge
operation serialization and path sandboxing, stepped-export `offlineMedia` recording, cloud
server upload bounds, xtask layer registration for `filmcraft-cloud`, layout-capture staleness,
and test coverage for limits/concurrency); the recovered UI-feedback round (Discord button
removed, web hamburger in Home's spot via the shared `menus::web_menu`, rounded dock cards
with per-viewport masking and focus ring, `--lgwin=24` brotli fix, craft-common-actions pinned
to `88148a57`) with two further review rounds fixing a native-window painting blocker and the
mask colour; and the refreshed web bundle built from that state. Run `git log --oneline
upstream/main..HEAD` for the current list; hashes in older notes are stale by design.

The optimized bundle is committed at
[`packaging/web/filmcraft-web-0.20.1.zip`](../packaging/web/filmcraft-web-0.20.1.zip) (inner WASM
39,689,696 bytes, 14,658,485 gzip; the zip now also carries the `.br` inside) and the precompressed
serving artifact at
[`packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br`](../packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br)
(9,389,752 bytes; the smallest measured HTTP transfer, served with `Content-Encoding: br`). This
bundle includes the `ui.layout.*` / `filmcraft.exportLayout()` feature, the rounded dock cards,
the web hamburger menu and the header without the Discord button — the previous zip predated all
of those. The local sandbox's last `git push` failed: the fork's GHPAT was rejected (expired or
rotated); the recovered commits live on the local `copilot/ui-feedback-recovery` branch until a
fresh token is provided. The fork also carries `.github/workflows/artifacts.yml` (tag-triggered):
it builds the unsigned macOS universal DMG (Kokoro voices, craft fonts, ad-hoc signed) plus the
web bundle and publishes them as a GitHub release with the workflow's own token — no Apple
certificate exists on the fork, so Gatekeeper needs an explicit first-launch open.

The release pipeline applies Binaryen `-O2` after wasm-bindgen when available. The size reduction
combines WebGPU-only dependencies and omission of optional Noto Serif in the browser; it is not a
per-module profile. A short local Node H.264 comparison measured warm O2 samples at about 477 ms
and O3 at about 485 ms; this is workload-specific, not a general benchmark. Browser scrubbing is
throttled and downscaled in code (Timeline scrub drags; see docs/web.md for the monitor-scrub-bar
scope note) but has not been profiled in a real browser. See
[web build docs](../docs/web.md) for current build and API details, and
[HANDOFF.md](HANDOFF.md) for the quality-gate record and the open items (browser smoke rerun,
pre-existing `proxies.rs:554` wasm clippy finding, and the upstream `craft-fcproj-format` rev
divergence). Toolchains, `target/` output, and any running preview servers are ephemeral across
sandboxes; artifacts are in git.

## References

- [WASM and bridge skill](skills/filmcraft-wasm-toolchain/SKILL.md)
- [Bridge capability contract and cloud PoC docs](../docs/web.md)
- [Static hosting requirements](../packaging/web/README.md)
- [Project architecture](../docs/architecture.md)
- [User-facing work lists](../ROADMAP.md), [gaps](../docs/gaps.md), and [target-app parity](../docs/target-app-parity.md)
