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

At the last handoff, branch `copilot/wasm-export-performance` tracked the same remote branch and
contained:

- `acb24658` (`wasm build`), which carries the earlier bridge/cloud/UI work.
- `4e088d9f` (`WASM: fix browser H.264 export`), which excludes the unsupported encoder thread-pool
  feature on WASM while keeping it for native builds, adds an H.264 export regression test, updates
  web size notes, and includes the optimized static bundle at
  [`packaging/web/filmcraft-web-0.20.1.zip`](../packaging/web/filmcraft-web-0.20.1.zip).

The artifact checksum at publication was
`03c3b49aab695b104151bc8d6aea43c4c0a7f03544d3e45eb022e81f05fdae9a`. The branch was pushed; no
pull request was opened and `main` was not changed. Confirm current remote state before relying on
these details.

The optimized bound WASM measured 42,139,756 bytes; the release pipeline applies Binaryen `-O2`
after wasm-bindgen when available. A short local Node H.264 comparison measured warm O2 samples at
about 477 ms and O3 at about 485 ms; this is a workload-specific result, not a general benchmark.
See [web build docs](../docs/web.md) for current build and API details.

## References

- [WASM and bridge skill](skills/filmcraft-wasm-toolchain/SKILL.md)
- [Bridge capability contract and cloud PoC docs](../docs/web.md)
- [Static hosting requirements](../packaging/web/README.md)
- [Project architecture](../docs/architecture.md)
- [User-facing work lists](../ROADMAP.md), [gaps](../docs/gaps.md), and [target-app parity](../docs/target-app-parity.md)
