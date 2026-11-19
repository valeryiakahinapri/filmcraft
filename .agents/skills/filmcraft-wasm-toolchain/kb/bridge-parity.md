# Auditing FilmCraft browser bridge parity

The bridge should expose the useful engine surface, not imitate native operating-system
capabilities. The current API and known boundaries are documented in
[docs/web.md](../../../../docs/web.md).

## Use a capability matrix

Compare every CLI mode and subcommand with the browser bridge. Record whether it is:

- Shared directly through the engine command registry.
- Supported with browser-native input/output semantics.
- Supported with a documented adaptation.
- Native-only because it needs paths, directory enumeration, process streams, sockets, or local
  devices.
- Missing and actionable.

Do not treat “all engine command IDs are listed” as complete parity. Check schemas, default
parameters, errors, asynchronous job completion, outputs, and persistence semantics too.

## Preserve the session and I/O contract

Keep headless scripting in an independent bridge session unless the user explicitly asks to mutate
the visible editor. Browser callers should pass `File`/bytes or registered virtual files rather
than native paths, and receive typed bytes/JSON rather than relying on CLI stdout or stderr.
Long-running work must pump jobs cooperatively and yield to the browser event loop; it must not
block the UI indefinitely.

## Compare behavior, not just symbols

The Node bridge test should load the real wasm-bindgen output and exercise representative operations
against the CLI where possible. Include error paths, hostile-size limits, media probe details,
project round trips, rendering, and an actual export whose bytes have a recognizable format
signature. Share parsing/probing code between CLI and WASM instead of duplicating format logic.

Browser smoke tests cover capabilities Node cannot establish, such as loading in a real browser,
GPU selection, audio, downloads, OPFS persistence, and user gestures. If no browser is installed,
report this as unverified rather than treating Node tests as a substitute.

## Keep boundaries explicit

The browser cannot safely or meaningfully reproduce arbitrary local filesystem paths, directory
scans, MCP stdio/TCP, CLI stdout/stderr, or native worker behavior. Document these exclusions and
offer a browser-native alternative only when it preserves the intended behavior. Apply strict
limits to imports, file counts, decoded frames, export duration, and allocations; propagate errors
instead of silently truncating or returning a success-shaped result.
