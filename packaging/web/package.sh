#!/usr/bin/env bash
# Build the browser version and zip it:  $DIST/filmcraft-web-<version>.zip
#
# Usage: packaging/web/package.sh [--skip-build]
#
# Needs: the wasm32-unknown-unknown target and the wasm-bindgen CLI at the crate's exact version
# (cargo install wasm-bindgen-cli --version <see xtask WASM_BINDGEN> --locked); `cargo xtask web` builds it. The zip holds a self-contained static site in filmcraft-web-<version>/ that works
# from any URL path and inside an <iframe>. Hosting notes: packaging/web/README.md.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/web"

if [[ "${1:-}" != "--skip-build" ]]; then
  # CI (the shared release workflow installs only trunk): install the wasm-bindgen CLI at the
  # crate's exact version. Locally, `cargo xtask web` says which version to install.
  if [[ -n "${CI:-}" ]]; then
    WASM_BINDGEN_VERSION="$(sed -n 's/^const WASM_BINDGEN: &str = "\(.*\)";$/\1/p' "$ROOT/xtask/src/main.rs")"
    if ! { command -v wasm-bindgen >/dev/null && wasm-bindgen --version | grep -qF "$WASM_BINDGEN_VERSION"; }; then
      cargo install wasm-bindgen-cli --version "$WASM_BINDGEN_VERSION" --locked
    fi
  fi
  (cd "$ROOT" && cargo xtask web)
fi

SITE="$CARGO_TARGET_DIR/web/dist"
[[ -f "$SITE/index.html" ]] || { echo "error: $SITE/index.html missing; run without --skip-build" >&2; exit 1; }
# Paths must be relative so the site works under any prefix.
if grep -Eq '(src|href)="/[^/]' "$SITE/index.html"; then
  echo "error: $SITE/index.html has root-absolute URLs; it would break when served from a sub-path" >&2
  exit 1
fi

NAME="filmcraft-web-$VERSION"
WORK="$CARGO_TARGET_DIR/web-package"
rm -rf "$WORK"
mkdir -p "$WORK/$NAME"
cp -R "$SITE/." "$WORK/$NAME/"
# Precompress the WASM for hosts that serve sibling .br files directly (Netlify, Cloudflare Pages
# with Brotli, nginx `brotli_static on`): quality 11 with the 16 MiB window reaches the smallest
# measured HTTP transfer for this build. Skipped (with a note) when the CLI is absent.
if command -v brotli >/dev/null; then
  brotli -q 11 --lgwin 24 -k "$WORK/$NAME/filmcraft_web_bg.wasm"
  echo "precompressed $NAME/filmcraft_web_bg.wasm.br:"
  ls -lh "$WORK/$NAME/filmcraft_web_bg.wasm.br"
else
  echo "note: brotli not found; the zip ships without a precompressed filmcraft_web_bg.wasm.br" >&2
fi
# Sample server configs (MIME type, caching, compression); harmless where unused.
cp "$HERE/_headers" "$HERE/.htaccess" "$WORK/$NAME/"
cp "$HERE/README.md" "$WORK/$NAME/HOSTING.md"
copy_docs "$WORK/$NAME"
rm -f "$DIST/$NAME.zip"
(cd "$WORK" && zip -qr9 "$DIST/$NAME.zip" "$NAME")
echo "wrote $DIST/$NAME.zip"
ls -lh "$DIST/$NAME.zip"
