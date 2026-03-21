#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh [TARGET...]
#
# Supported targets:
#   x86_64-unknown-linux-gnu
#   aarch64-apple-darwin
#   x86_64-apple-darwin
#   x86_64-pc-windows-gnu
#
# If no targets given, builds for the current platform.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"

# ── Detect default target ────────────────────────────────────────────────────

default_target() {
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)  echo "x86_64-unknown-linux-gnu" ;;
    Darwin-arm64)  echo "aarch64-apple-darwin" ;;
    Darwin-x86_64) echo "x86_64-apple-darwin" ;;
    *)             echo "x86_64-unknown-linux-gnu" ;;
  esac
}

if [[ $# -gt 0 ]]; then
  TARGETS=("$@")
else
  TARGETS=("$(default_target)")
fi

# ── Build frontend (once) ───────────────────────────────────────────────────

"$ROOT_DIR/scripts/build-frontend.sh"

# ── Build each target ────────────────────────────────────────────────────────

for TARGET in "${TARGETS[@]}"; do
  echo "──────────────────────────────────────────────"
  echo "Building target: $TARGET"
  echo "──────────────────────────────────────────────"

  rustup target add "$TARGET" 2>/dev/null || true
  cargo build --release --target "$TARGET" -p lsp-skill-server -p lsp-skill-cli

  # ── Assemble bundle ──────────────────────────────────────────────────────

  OUT_DIR="$ROOT_DIR/target/release-bundle/$TARGET"
  rm -rf "$OUT_DIR"
  mkdir -p "$OUT_DIR"

  # Binaries
  if [[ "$TARGET" == *windows* ]]; then
    cp "$ROOT_DIR/target/$TARGET/release/lsp-skill-server.exe" "$OUT_DIR/"
    cp "$ROOT_DIR/target/$TARGET/release/lsp-skill.exe" "$OUT_DIR/"
  else
    cp "$ROOT_DIR/target/$TARGET/release/lsp-skill-server" "$OUT_DIR/"
    cp "$ROOT_DIR/target/$TARGET/release/lsp-skill" "$OUT_DIR/"
  fi

  # Dist files
  cp "$DIST_DIR/.env.example"     "$OUT_DIR/"
  cp "$DIST_DIR/AGENTS-skills.md" "$OUT_DIR/"
  cp "$DIST_DIR/AGENTS-mcp.md"    "$OUT_DIR/"
  cp -R "$DIST_DIR/skills"         "$OUT_DIR/skills"

  # Quick-start guide (HTML + pics)
  if [[ -f "$DIST_DIR/README.html" ]]; then
    cp "$DIST_DIR/README.html" "$OUT_DIR/"
  fi
  if [[ -d "$DIST_DIR/pics" ]] && ls "$DIST_DIR/pics"/* &>/dev/null; then
    mkdir -p "$OUT_DIR/pics"
    cp "$DIST_DIR/pics"/* "$OUT_DIR/pics/"
  fi

  # ── Package ──────────────────────────────────────────────────────────────

  (
    cd "$ROOT_DIR/target/release-bundle"
    if [[ "$TARGET" == *windows* ]]; then
      zip -rq "lsp-skill-$TARGET.zip" "$TARGET"
      echo "Created: target/release-bundle/lsp-skill-$TARGET.zip"
    else
      tar -czf "lsp-skill-$TARGET.tar.gz" "$TARGET"
      echo "Created: target/release-bundle/lsp-skill-$TARGET.tar.gz"
    fi
  )
done
