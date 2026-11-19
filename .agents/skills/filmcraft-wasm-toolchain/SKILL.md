# FilmCraft Rust/WASM toolchain and bridge

Use this skill when building FilmCraft for `wasm32-unknown-unknown`, changing the browser
JavaScript API, validating release size/performance, or packaging the static web app.

## Read the source of truth first

- [Web build, runtime, JS APIs, cloud PoC, and test instructions](../../../docs/web.md)
- [Web hosting and static-server requirements](../../../packaging/web/README.md)
- [CLI command and control-protocol docs](../../../docs/control-protocol.md)
- [Workspace layering and architecture](../../../docs/architecture.md)
- [Contributor/agent workflow](../../../docs/agents.md)

The committed Rust toolchain file, manifests, `xtask`, and those docs define current versions and
commands. Do not rely on toolchain paths, downloads, generated `target/` files, or running servers
from an earlier sandbox; install/activate the toolchain again in the current environment.

## Build and validate

1. Use the repository wrapper, not a separately invented web build:
   - `cargo xtask web --dev` for fast iteration.
   - `cargo xtask web` for the release distribution in `target/web/dist`.
   - `cargo xtask wasm` for the workspace's WASM compatibility check.
2. The wasm-bindgen CLI must match the crate version exactly. `cargo xtask web` checks this.
3. Keep the dev profile intact when comparing release output. Release optimization settings are
   separate; inspect `[profile.release]` and measure before changing either profile.
4. `cargo xtask web` runs `wasm-bindgen` first, then optionally `wasm-opt -O2` on the generated,
   bound `filmcraft_web_bg.wasm`. Do not optimize the raw Rust module before wasm-bindgen: that
   can remove metadata the binder needs. Binaryen is optional, so report whether it was used.
5. Test the actual generated WASM through
   `apps/filmcraft-web/tests/js-bridge.mjs`; a Rust compile alone does not validate JS glue,
   imports/exports, command behavior, or media bytes. Run the browser smoke test only when its
   required browser tooling is available. Report browser validation separately from Node tests.
6. Confirm the final package with the repository packaging script, archive validation, and an
   HTTP check that `.wasm` is served as `application/wasm`. Do not claim a server remains hosted
   across sandboxes; processes and ignored build output are ephemeral.

See [release-build and measurement workflow](kb/release-build-and-measurement.md) for profiling
and fair speed/size comparisons.

## Keep native and WASM capabilities correctly separated

- A browser build is single-threaded unless the build explicitly enables WASM atomics/threads and
  the deployment meets their requirements. Cross-origin isolation headers alone do not add
  atomics support.
- Do not enable a native Rayon/thread-pool feature globally when the implementation creates a
  worker pool that `wasm32-unknown-unknown` cannot support. Use a target-specific Cargo dependency
  feature to preserve native parallelism while omitting that feature on WASM; inspect the resolved
  feature graph for both targets after changing it.
- Browser exports and other long operations should use the existing bounded/cooperative job
  pumping path. Do not move work into a thread or silently substitute a success-shaped fallback.
- WASM code must not assume native filesystem paths, process IDs, system wall-clock APIs, thread
  sleep/spawn, or unrestricted memory. Follow the platform-specific helpers and limits documented
  in `docs/web.md`.

The native binary remains a separate target: the user's request is for a headless Linux CLI /
engine path, not a desktop GUI. Keep any native threading/performance work in the appropriate
native target and validate it independently.

## Audit the bridge against CLI behavior

`FilmCraftBridge` is an independent engine session; it is not the visible editor's session.
Use the shared engine command registry wherever possible, then audit CLI modes one by one:

1. Compare CLI commands, descriptions, JSON-Lines execution, inspection, import/probe/render/
   export, and job behavior against the bridge.
2. Expose browser-feasible operations with equivalent parameter validation and useful output;
   test that exports and reads return the expected bytes, not just a successful status.
3. Treat filesystem paths, directory discovery, process streams, native servers, and local-device
   access as host capabilities. Do not pretend these exist in the browser; provide a browser
   equivalent only where semantics are clear, and document unavoidable gaps.
4. Reuse shared probe/engine logic rather than maintaining subtly different CLI and web behavior.
   Exercise CLI and WASM outputs against the same fixtures/oracle where possible.
5. Bound input bytes, file counts, decode frames, allocations, recursion, and operation time.
   Invalid or unsupported input must reject/report an error; it must not crash or be reported as
   success.

See [bridge parity and browser capability boundaries](kb/bridge-parity.md) and the bridge section
of `docs/web.md` for the current API contract.

## Hosting and media storage

The shipped web distribution is static; cloud storage is a separate optional backend. Follow
`docs/web.md` and `packaging/web/README.md` for the current PoC contract. Keep that PoC loopback
only unless authentication, authorization, transport security, quotas, and storage isolation are
implemented and reviewed. Do not present it as a production SaaS service.

For browser proxy caching, key cached content by immutable source and proxy versions (for example
ETags/content hashes), not by filename alone. Revalidate versions before reuse, handle browser
storage quota/eviction, and make stale proxy invalidation explicit. A server-side transcode or
headless render is a separate service feature; do not imply the static web package provides one.

## Asset and clean-room constraints

Follow `AGENTS.md`: do not copy PhotoCraft implementation or artwork; use FilmCraft's own theme
tokens and original assets. New assets require an approved license, sidecar, and attribution
entry. Preserve the Rust safety rules, bounded hostile-input handling, layering, and `cargo xtask
ci` gates.
