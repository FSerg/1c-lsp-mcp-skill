#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/target/frontend-dist"
FRONTEND_DIR="$ROOT_DIR/crates/frontend"
DX_PUBLIC_DIR="$ROOT_DIR/target/dx/lsp-skill-frontend/release/web/public"
DX_ROOT="$ROOT_DIR/.tools/dx-0.6.3"
DX_BIN="$DX_ROOT/bin/dx"

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

if [[ ! -x "$DX_BIN" ]]; then
  cargo install dioxus-cli --version 0.6.3 --locked --root "$DX_ROOT"
fi

(
  cd "$FRONTEND_DIR"
  npm ci
  npm run build:css
  rm -rf "$DX_PUBLIC_DIR"
  "$DX_BIN" build \
    --platform web \
    --package lsp-skill-frontend \
    --bin lsp-skill-frontend \
    --release
)

cp -R "$DX_PUBLIC_DIR"/. "$DIST_DIR"/
mkdir -p "$DIST_DIR/assets"
cp "$FRONTEND_DIR/assets/app.css" "$DIST_DIR/assets/app.css"
perl -0pi -e 's#</head>#    <link rel="stylesheet" href="/assets/app.css"></head>#' "$DIST_DIR/index.html"
