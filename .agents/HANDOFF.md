# FilmCraft session handoff

This note records the seam between the completed WASM packaging/fix work and a fresh sandbox. It is
not the project roadmap; derive the next work from the user's latest request and the public
worklists. See [agent context](CONTEXT.md) and the [WASM/bridge skill](skills/filmcraft-wasm-toolchain/SKILL.md)
for durable context and operating guidance.

## Published state to recover

- The feature branch is `copilot/wasm-export-performance`; current `HEAD` and
  `origin/copilot/wasm-export-performance` both point to `a67eb08ec9efe7734e1fda2c4080abbd79da4623`.
  The rounded UI, browser hamburger/cascading menus, WebGPU-only dependency selection, and scrub
  tuning are still **uncommitted worktree changes**. No PR has been opened and `main` is unchanged.
- The updated optimized distribution is present in the worktree at
  [`packaging/web/filmcraft-web-0.20.1.zip`](../packaging/web/filmcraft-web-0.20.1.zip), SHA-256
  `2950dee0ad945ecc7e3bd060c27b00f2925fe86e8889c67f58e907d3753f298e`. It is not yet committed or
  pushed.
- The rebuilt static app is currently serving at `http://127.0.0.1:8765/`; its WASM responds `200`
  with `application/wasm` and content length 39,126,617. It serves the temporary
  `/tmp/filmcraft-target/web/dist`, so it will not survive a fresh sandbox. This is only a local
  preview, not the public web app/cloud deployment requested earlier.
- The base shell does not have Rust on `PATH`. An isolated Rust 1.100.0 toolchain plus
  `wasm32-unknown-unknown`, `rustfmt`, and clippy was installed under `/tmp/filmcraft-*` for
  validation in this session; it will not survive a fresh sandbox. Matching wasm-bindgen CLI
  0.2.129 and Binaryen 133 were also installed temporarily and used for the final release package.
  Do not rely on old `/tmp` outputs or a previous server.

## Work completed and verified

- The final browser release WASM was rebuilt from the current source and processed with Binaryen
  v133 `wasm-opt -O2`. It measures **39,126,617 bytes** raw and **14,516,260 bytes gzip**, 7.15%
  smaller raw and 7.57% smaller gzip than the preceding optimized build.
- The rebuilt ZIP is **14,561,456 bytes**; its SHA-256 is recorded above and `unzip -t` passed.
  The artifact still needs to be committed/pushed with the code changes.
- In the earlier equipped environment, the JS bridge E2E against release WASM passed (CLI command
  parity, JSON-Lines, rendering, import/probe, export including H.264 MP4, and project round-trip).
  In this session, all UI unit tests passed again (365 passed, one ignored); the browser scrub
  regression passed; `cargo fmt --all -- --check` passed; native UI clippy passed; and the
  `filmcraft-ui-egui` WASM-target `cargo check` passed. The freshly built release WASM also passed
  the JS bridge E2E (CLI parity, JSON-Lines, rendering, import/probe, exports including H.264, and
  project round-trip).
- The WASM-target clippy check fails on the unchanged `drop(output)` in
  `crates/engine/src/proxies.rs:554` (`clippy::drop_non_drop`). Native clippy passes after the
  browser menu/scrub helper extractions. A first UI test attempt ran out of space in
  `/workspaces`; it was rerun successfully with `CARGO_TARGET_DIR=/tmp/filmcraft-target`.
  `target/debug/incremental` was removed (generated compiler cache only) to restore 4.4 GiB to the
  workspace volume.
- The visible-editor layout profile is now implemented in the uncommitted changes. It exposes
  `ui.layout.export` / `ui.layout.apply` and `filmcraft.exportLayout()` /
  `filmcraft.applyLayout(layout, options?)`. The versioned JSON captures the docking model and
  layout-related panel state, validates before mutation, scales floating geometry with the
  viewport, and can save the dock tree to an existing named workspace. See the protocol and web
  docs for the exact included/excluded state.
- Latest validation: `cargo fmt --all -- --check`, all 369 `filmcraft-ui-egui` library tests
  (one ignored), native UI clippy with `-D warnings`, and
  `cargo check --locked -p filmcraft-web --target wasm32-unknown-unknown` passed. The focused
  control-channel test also verified applying serialized JSON and saving to the Color workspace.
  `git diff --check` passed. The release WASM bundle and browser smoke test have not been rebuilt
  or rerun after the layout-profile changes; no browser executable is available in this
  environment. A final optimized build was attempted in temporary storage, but the prior isolated
  Rust toolchain under `/tmp/filmcraft-*` had disappeared (`cargo: command not found`); no release
  build result was produced.
- The WebGPU-only feature graph omitted WebGL; checks found no `pollster` or `tempfile` in the
  normal WASM graph. The native off-screen export renderer remains native-only.
- Browser Timeline scrubbing is capped at 15 preview requests/second and half scale, uses a 6 ms
  cooperative pump budget, suppresses scrub audio, and requests normal paused resolution on
  release. This is code-level tuning only: no real browser was available to measure perceived
  latency or verify the rendered UI.
- Existing `effect_color_hex_ui` integration failures were reproduced in a clean baseline at the
  starting commit; they predate this change.
- Initial clippy runs reported UI cognitive-complexity thresholds; helper extractions now pass
  native UI clippy. WASM clippy still fails on the unchanged `drop(output)` in
  `crates/engine/src/proxies.rs:554` (`clippy::drop_non_drop`).
- No browser executable was available. No browser visual/scrub test or public deployment was
  completed.

## Immediate seam for the next session

1. Recheck the branch and worktree. The isolated toolchain and build outputs are ephemeral. The
   latest layout-profile source passes focused/full UI tests, formatting, native clippy, and the
   web-target check, but its release WASM bundle and browser smoke have not been rerun. Keep the
   unrelated `proxies.rs:554` WASM clippy finding distinct.
2. Use an actual browser if available for layout-profile round-trip, menu, rounded-panel, WebGPU,
   and scrub smoke tests; visual/performance verification is still open.
3. Do not commit/push until the repository quality gate is green; never push to `main`.
4. Continue the still-open earlier goal: a genuinely hosted web app/cloud workspace demo with an
   immediately openable video/audio/proxy project and a headless native cloud export path. The
   existing cloud PoC is loopback-only and unauthenticated; it is not suitable for public exposure.
   Keep headless work distinct from desktop GUI work.

No separate new PLAN was created: the remaining product direction is already represented by the
repository's roadmap/gap/parity documents. Do not treat this handoff as the full next-session scope.
