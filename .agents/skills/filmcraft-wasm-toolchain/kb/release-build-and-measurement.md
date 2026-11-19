# FilmCraft WASM release build and measurement

This note explains how to produce meaningful release-size and performance evidence for FilmCraft.
The live command/version details remain in [docs/web.md](../../../../docs/web.md) and
[xtask](../../../../xtask/src/main.rs).

## Build pipeline

`cargo xtask web` builds `filmcraft-web` for `wasm32-unknown-unknown`, checks the exact
wasm-bindgen CLI version, binds the module for the web target, optionally applies Binaryen
`wasm-opt -O2`, and writes the static distribution to `target/web/dist`. Keep this order:

1. Rust compilation.
2. `wasm-bindgen` generation.
3. Optional `wasm-opt` on the generated `filmcraft_web_bg.wasm`.
4. Static assets and cache-busting build hash generation.

Optimizing the raw Rust WASM before binding can strip metadata the binder requires. Do not replace
the repository pipeline with an optimization command that skips web glue or stale-cache handling.

## Compare profiles and optimizers fairly

- Cargo's dev and release profiles answer different questions. Preserve the dev profile when
  optimizing release output; use release profile overrides only for controlled experiments.
- Record the Rust/Cargo and wasm-bindgen versions, target, profile flags, Binaryen version and
  arguments, input/output byte counts, and whether stripping or compression is included.
- Measure both download/packaged size and executable size. Distinguish raw WASM, gzip/Brotli WASM,
  static distribution, and ZIP sizes.
- For speed comparisons, run representative real work through the generated WASM in the same
  runtime. Warm up first, repeat samples, compare medians or distributions, and keep workload,
  resolution, frame count, codec, input and runtime fixed. `wasm-opt -O3` or a smaller artifact is
  not automatically faster.
- Run the bridge end-to-end suite against the exact artifact being measured. A successful link or
  optimizer exit is not evidence that the bound module still works.

## Attribute size before removing features

Use dependency/features inspection and a WASM profiler (for example, Twiggy) on the final bound
artifact. Confirm that a candidate is actually included and estimate its contribution before
removing it. Then test all affected fallbacks and user-visible capabilities. Rendering fallbacks,
codecs, fonts, and UI functionality can be expensive but intentional; do not trade them away based
only on symbol size. Report measured deltas, not speculative savings.

## Browser-specific failure modes

A web build may compile while still failing at runtime because a native dependency tries to create
a thread pool or uses an unavailable OS API. Validate each browser operation end-to-end. If a
native feature is unsupported on WASM, condition it by target rather than disabling it globally,
and verify the resolved dependency features for both native and WASM builds.

The browser may be single-threaded even when the page is cross-origin isolated. Threaded WASM
requires an atomics-capable build and a compatible runtime/deployment. Do not infer threaded
support from COOP/COEP headers.
