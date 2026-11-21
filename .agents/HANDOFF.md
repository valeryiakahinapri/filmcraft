# FilmCraft session handoff

This note records the seam between the completed WASM packaging/fix/review work and a fresh
sandbox. It is not the project roadmap; derive the next work from the user's latest request and
the public worklists. See [agent context](CONTEXT.md) and the
[WASM/bridge skill](skills/filmcraft-wasm-toolchain/SKILL.md) for durable context and operating
guidance.

## Published state

- The feature branch is `copilot/wasm-export-performance`, rebased onto `storytold/filmcraft`
  `main` (upstream commits up to the slip-monitor merge are included) and pushed to
  `valeryiakahinapri/filmcraft`. Run `git log --oneline upstream/main..HEAD` for the current
  commit list; do not trust hashes recorded in notes.
- The optimized distribution is committed at
  [`packaging/web/filmcraft-web-0.20.1.zip`](../packaging/web/filmcraft-web-0.20.1.zip)
  (14,561,456 bytes; SHA-256 `2950dee0ad945ecc7e3bd060c27b00f2925fe86e8889c67f58e907d3753f298e`;
  inner `filmcraft_web_bg.wasm` is 39,126,617 bytes, SHA-256
  `fd9051456d934eee007282e0676fa15f177d0c0d7c45a6c1c84767d5c452cc7c`). **Known gap:** this bundle
  was built before the `ui.layout.*` / `filmcraft.exportLayout()` feature landed, so it does not
  contain the layout-profile API; rebuild with `cargo xtask web` +
  `packaging/web/package.sh` before serving a bundle that must include it.
- The precompressed serving artifact is committed at
  [`packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br`](../packaging/web/filmcraft-web-0.20.1/filmcraft_web_bg.wasm.br):
  9,314,350 bytes (Brotli quality 11, 16 MiB window / lgwin 24), SHA-256
  `8487a26883c3655f0cf561efdbdfd266ffd350d0afb028b29b192bd171a1327a`, round-trip verified
  byte-for-byte against the zip's WASM. This is the smallest measured HTTP transfer for this
  build (servers send it with `Content-Encoding: br`). `packaging/web/package.sh` now emits the
  `.br` when the `brotli` CLI is available.
- A full review-vs-main pass ran over the branch (findings fixed in the "review fixes" commit):
  the bridge serializes operations and sandboxes paths, stepped wasm exports record
  `offlineMedia`, the cloud server bounds stalls/concurrency/uploads, the xtask layers gate
  covers `filmcraft-cloud`, and layout capture tolerates stale closed-float entries. Details in
  the commit message.

## Quality gates that ran on this branch

- `cargo fmt --all -- --check`, `cargo xtask layers`, `cargo metadata` (lock consistent).
- `cargo check -p filmcraft-web --target wasm32-unknown-unknown` (whole web graph).
- Native `cargo check` for engine, codecs, text, cli, ui-egui, export, gpu, cloud.
- `cargo test -p filmcraft-cloud` (4 integration tests).
- See `git log` and the review-fix commit for the rest; re-run gates for the crates you touch —
  they are listed in AGENTS.md `## This repository`.

Still open, inherited from earlier sessions:

- `export_tests::every_builtin_preset_exports_and_ffprobe_confirms` fails only where ffprobe
  lacks an APV decoder (this sandbox's FFmpeg 7.1.5 has none; the assertion sees `codec_name:
  null`). Environment-dependent, not a branch defect; everything else in the engine suite
  passes.
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
2. If a served bundle must include `ui.layout.*`, rebuild the zip and `.br` from HEAD and update
   the sizes/SHA-256s in `packaging/web/README.md` and `docs/web.md`.
3. Do not push to `main` until the repository quality gate is green (see AGENTS.md rule 3).
4. Keep the still-open earlier goal in mind: a genuinely hosted web app/cloud workspace demo. The
   cloud PoC is loopback-only and unauthenticated; it is not suitable for public exposure. Keep
   headless work distinct from desktop GUI work.

No separate new PLAN was created: the remaining product direction is already represented in the
repository's roadmap/gap/parity documents. Do not treat this handoff as the full next-session
scope.
