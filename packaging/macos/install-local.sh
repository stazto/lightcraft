#!/usr/bin/env bash
# Assemble Lightspot.app from the release build and put it in /Applications (or the folder given),
# for this machine only: no signing, no DMG, no notarization (those live in package.sh). Build first:
#   cargo build --release -p lightcraft -p lightcraft-cli
#   packaging/macos/install-local.sh [/path/to/Lightspot.app]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP="${1:-/Applications/Lightspot.app}"
BIN="$ROOT/target/release/lightcraft"
[[ -x "$BIN" ]] || { echo "error: $BIN not built (cargo build --release -p lightcraft)" >&2; exit 1; }
VERSION=$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')
SHORT=${VERSION%%-*}
SHA=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/LightCraft"
cp "$ROOT/assets/app-icon/lightcraft.icns" "$APP/Contents/Resources/LightCraft.icns"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT/g" -e "s/@BUILD_SHA@/$SHA/g" \
  "$ROOT/packaging/macos/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null
# ad-hoc signature so Gatekeeper on Apple silicon runs the bundle; the binary itself is already ad-hoc signed by the linker
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
echo "installed $APP (version $VERSION, $SHA)"
