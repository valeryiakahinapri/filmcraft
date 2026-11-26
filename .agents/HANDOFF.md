# FilmCraft session handoff

This note records the seam between the completed WASM packaging/fix/review work and a fresh
sandbox. It is not the project roadmap; derive the next work from the user's latest request and
the public worklists. See [agent context](CONTEXT.md) and the
[WASM/bridge skill](skills/filmcraft-wasm-toolchain/SKILL.md) for durable context and operating
guidance.

## Published state

- The feature branch is `copilot/wasm-export-performance`, rebased onto `storytold/filmcraft`
  `main` (upstream commits up to the slip-monitor merge are included). **The fork's GHPAT was
  rejected on 26 Nov 2026 (`remote: Invalid username or token`; the REST API says `Bad
  credentials`) — nothing since the 21 Nov review-fixes commit is pushed.** The recovery work
  lives on the local branch `copilot/ui-feedback-recovery` (based on the pushed `main` tip) and
  is merged into local `main` with tag `build-*` ready to push:
  1. `098f6c8c` — the recovered UI-feedback round (see below).
  2. `ac2d981a` — review fixes: a native floating window's corner masks painted into the ROOT
     viewport at child-local coordinates (egui runs immediate-viewport callbacks re-entrantly
     inside `show_content`, and `layer_painter` always targets the current viewport). Bodies
     now record their viewport; `dock_area` paints only the ROOT pass's, `window_drawn` paints
     the window's own at the end of its pass. The mask colour is per-body (`app_bg` docked,
     `panel_bg` floating). The wasm hamburger is the one tested `menus::web_menu`.
  3. `4134795d` — round-2 notes: `timeline_drop_target` skips native-window bodies, the
     `menu_bar` kittest test is native-only, the web-menu child Ui has a stable id.
  4. `f8236139` — `.github/workflows/artifacts.yml`: tag-triggered unsigned macOS universal +
     web bundle + GitHub release (needs Actions enabled on the fork).
  5. the refreshed artifacts commit — new zip + `.br` + docs (below). When a fresh token
     arrives: `git push origin main && git push origin <build-tag>` (or the branch first).
- The refreshed optimized distribution is committed at
  [`packaging/web/filmcraft-web-0.20.1.zip`](../packaging/web/filmcraft-web-0.20.1.zip)
  (24,124,589 bytes; SHA-256
  `518953c12ee78eef09104a04c0e94731ee4a17f0cdf0487cb4061810bbeede39`;
  inner `filmcraft_web_bg.wasm` is 39,689,696 bytes, SHA-256
  `4ca52ab7cdb9aa0f485e04268bea5d40c2731d89b60960bbf9b0291763ec9b7c`), built from the recovered
  branch state, so it now contains `ui.layout.*`, the rounded dock cards, the web hamburger
  menu and the Discord-free header, and the zip also carries the `.br` inside.
- The precompressed serving artifact is committed at
  [`packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br`](../packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br):
  9,389,752 bytes (Brotli quality 11, 16 MiB window / lgwin 24), SHA-256
  `5ba94a8546362c5ef8d1c708bd9f1c8595a7af870bfeae5c5acbdee14b2f1fd2`, round-trip verified
  byte-for-byte against the zip's WASM. This is the smallest measured HTTP transfer for this
  build (servers send it with `Content-Encoding: br`).
- A full review-vs-main pass ran over the branch (findings fixed in the "review fixes" commit):
  the bridge serializes operations and sandboxes paths, stepped wasm exports record
  `offlineMedia`, the cloud server bounds stalls/concurrency/uploads, the xtask layers gate
  covers `filmcraft-cloud`, and layout capture tolerates stale closed-float entries. Details in
  the commit message.

## Quality gates that ran on this branch

- `cargo fmt --all -- --check`, `cargo xtask layers`, `cargo metadata` (lock consistent).
- `cargo check -p filmcraft-web --target wasm32-unknown-unknown` (whole web graph, including the
  `BridgeServices` confinement wrapper).
- Native `cargo check` for engine, codecs, text, cli, ui-egui, export, gpu, cloud.
- `cargo test -p filmcraft-cloud` (4 integration tests), `--lib` suites for ui-egui (377 passed,
  1 ignored), codecs (188), engine (924; see the APV note below), text (57), gpu (55), cli.
- `cargo clippy --all-targets -- -D warnings` for cloud, codecs, engine, text, xtask, gpu, cli,
  ui-egui, web (native). The one fix needed: the layout-preset round-trip test's
  `Default::default()` field reassignment (branch pushes do not run CI, so it had never fired).
- The full Node `js-bridge.mjs` e2e against freshly bound debug WASM: CLI command parity, JSON
  Lines run, limits (pumpJobs bounds, 10000-line/8 MiB script caps, 16 MiB params, 1024-file
  import cap), path confinement rejections, busy-guard rejection and release, render, batch
  import, media probes, PNG and H.264 exports through the `BridgeServices` wrapper, and the
  project round-trip — all pass.

Four review rounds ran over the whole branch diff (self review + delegated reviewers): rounds
1-3 fixed ~35 findings across the bridge (concurrency, caps, IO-layer confinement), the engine
stepped-export `offlineMedia` recording, the cloud server (upload admission/limits/timeouts/
sweeps), the cloud client (manifest validation, response caps), UI (stale layout capture,
translated menu label, keyboard test, wasm test gating), the build graph (xtask layers for
filmcraft-cloud, native-only `threads` mirrors, Noto Serif gating), and packaging (Brotli
pipeline, caching rules, docs). Round 4 approved the final state for merge.

Still open, inherited from earlier sessions:

- `export_tests::every_builtin_preset_exports_and_ffprobe_confirms` fails only where ffprobe
  lacks an APV decoder (this sandbox's FFmpeg 7.1.5 has none; the assertion sees `codec_name:
  null`). Environment-dependent, not a branch defect; everything else in the engine suite
  passes. Similarly, cli's `workflows::phone_vfr_clip_extract_gain_title_to_h264_mp4` compares
  frames against the local ffmpeg oracle and failed in this sandbox only (no native-path branch
  code changed; likely the same ffmpeg-version class).
- `file.projectManager` does host IO with raw `std::fs` instead of `Services` (pre-existing;
  fails closed on wasm32, unreachable through the bridge) — follow-up: route it through
  `Services` or disable it on wasm.
- The browser smoke test (`apps/filmcraft-web/tests/smoke.mjs`, needs a browser via CDP) has not
  been rerun against a bundle rebuilt with `ui.layout.*`; the committed zip predates that feature.
- WASM-target clippy still fails on the unchanged `drop(output)` in
  `crates/engine/src/proxies.rs:554` (`clippy::drop_non_drop`) — pre-existing, not this branch.
- `craft-fcproj-format` sits at rev `3af2280…` while the other craft-libs pins are at `dc6adc3f…`
  — pre-existing on upstream `main`, not introduced here.
- The cloud PoC remains loopback-only and unauthenticated; do not expose it publicly.

## Immediate seam for the next session

1. Recheck the branch and worktree; toolchains and `target/` output are ephemeral, artifacts are
   in git now.
2. The recovered bundle was rebuilt locally (wasm-opt -O2 + Brotli q11 lgwin 24) and its rounded
   corners were verified visually: `FILMCRAFT_UI_SNAPSHOT_DIR` renders with Mesa's lavapipe
   software Vulkan ICD (extracted from the distro .deb with `dpkg -x`, no sudo needed; set
   `VK_ICD_FILENAMES` + `LD_LIBRARY_PATH`), and the focus-*.png snapshots show the arc-following
   ring with no corner artifacts in both themes. The sandbox has no GPU otherwise.
3. A fresh GHPAT is the single blocker left: push `main` (+ `copilot/wasm-export-performance`,
   + tag `build-20261126-ffeb9837`), confirm Actions is enabled on the fork (the REST API with a
   repo-scoped token can enable it), and the tag push runs `.github/workflows/artifacts.yml`,
   which publishes the unsigned macOS universal DMG and the web bundle as a GitHub release.
4. Keep the still-open earlier goal in mind: a genuinely hosted web app/cloud workspace demo. The
   cloud PoC is loopback-only and unauthenticated; it is not suitable for public exposure. Keep
   headless work distinct from desktop GUI work.

No separate new PLAN was created: the remaining product direction is already represented in the
repository's roadmap/gap/parity documents. Do not treat this handoff as the full next-session
scope.
