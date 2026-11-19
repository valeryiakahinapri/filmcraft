# FilmCraft session handoff

This note records the seam between the completed WASM packaging/fix work and a fresh sandbox. It is
not the project roadmap; derive the next work from the user's latest request and the public
worklists. See [agent context](CONTEXT.md) and the [WASM/bridge skill](skills/filmcraft-wasm-toolchain/SKILL.md)
for durable context and operating guidance.

## Published state to recover

- Fetch `origin/copilot/wasm-export-performance`; at handoff it pointed to
  `4e088d9f145e9b46c62211b382f5892fa09017b9`. The branch includes the earlier `acb24658` base and
  the WASM export fix. Verify branch/remote status before editing.
- The optimized web distribution is committed at
  [`packaging/web/filmcraft-web-0.20.1.zip`](../packaging/web/filmcraft-web-0.20.1.zip), SHA-256
  `03c3b49aab695b104151bc8d6aea43c4c0a7f03544d3e45eb022e81f05fdae9a`.
- No PR was opened. `main` was not pushed to or modified by the packaging commit.
- The static app was responding at `http://127.0.0.1:8765/` immediately before this handoff, with
  the WASM returned as `application/wasm`. This is a loopback development server, not a public
  deployment; its process will not survive the sandbox change.
- The previous local web server, build outputs, and the external toolchain activation files do not
  survive this sandbox boundary. Reinstall/activate tools from checked-in project instructions;
  do not rely on `/workspaces/.filmcraft-toolchain` or any `/tmp`/`target` artifact.
- The last verified environment used Rust 1.100.0, `wasm-bindgen-cli` 0.2.129, and Binaryen 133.
  The web build checks the wasm-bindgen version against its crate; install the matching version
  rather than assuming this machine's CLI version.

## Work completed and verified

- `craft-video/threads` is enabled only for non-WASM builds. This fixes the browser H.264 error
  `cannot build the 1-thread pool: operation not supported on this platform` without removing
  native encoder parallelism.
- Added a JS bridge regression that performs an actual short H.264 export and checks the MP4
  `ftyp` signature.
- Built the release web app with Rust's release settings and Binaryen v133 `wasm-opt -O2` after
  wasm-bindgen. The resulting WASM measured 42,139,756 bytes; the ZIP archive test passed.
- The JS bridge E2E harness passed on O2 and O3 artifacts, including the H.264 regression. Native
  H.264 export test passed; its optional ffprobe subcheck was skipped because ffprobe was absent.
- `cargo fmt --all -- --check`, `cargo metadata --locked --no-deps`, `git diff --check`, ZIP
  validation, and HTTP checks for the served WASM (`200`, `application/wasm`) passed.
- O2 was about 2% faster than O3 in a small warm Node export comparison. Do not generalize this
  result; use representative workloads for future performance decisions.
- No browser executable was available, so visual/browser smoke validation was not done.

## Immediate seam for the next session

1. Confirm the branch, clean worktree, remote head, and tracked ZIP from GitHub before rebuilding.
2. Read `docs/web.md`, `packaging/web/README.md`, and the WASM skill/KB before changing the build
   or bridge.
3. Continue the still-open product goal from the user's prior messages: a genuinely hosted web app
   and cloud workspace demo with an immediately openable project containing video/audio/proxy
   media, and a headless native cloud export path for source-resolution lossless output. The
   existing cloud PoC is loopback-only and unauthenticated; it does not by itself satisfy public
   hosting or production security. Verify which requested pieces already exist before implementing
   them.
4. Keep the headless requirement distinct from desktop GUI work. Browser visual testing may need
   a browser installation or a real hosted browser environment; never claim it ran without evidence.

No separate new PLAN was created: the remaining product direction is already represented by the
repository's roadmap/gap/parity documents. Do not treat this handoff as the full next-session scope.
