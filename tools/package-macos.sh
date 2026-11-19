#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BIN_PATH=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin)
      BIN_PATH="${2:-}"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
if [[ -z "$VERSION" ]]; then
  echo "Failed to read version from Cargo.toml" >&2
  exit 1
fi

if [[ -z "$BIN_PATH" ]]; then
  if [[ -f "target/universal/release/island" ]]; then
    BIN_PATH="target/universal/release/island"
  else
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
    cargo build --release --locked --target aarch64-apple-darwin
    cargo build --release --locked --target x86_64-apple-darwin
    mkdir -p target/universal/release
    lipo -create \
      target/aarch64-apple-darwin/release/island \
      target/x86_64-apple-darwin/release/island \
      -output target/universal/release/island
    BIN_PATH="target/universal/release/island"
  fi
fi

if [[ ! -f "$BIN_PATH" ]]; then
  echo "Binary not found at: $BIN_PATH" >&2
  exit 1
fi

DIST="$ROOT/dist"
mkdir -p "$DIST"
find "$DIST" -maxdepth 1 -type f \( -name "*.dmg" -o -name "*.zip" \) ! -name "island-${VERSION}-*" -delete

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

APP_DIR="$STAGE/Kite Island.app"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/island"
chmod 755 "$APP_DIR/Contents/MacOS/island"
sed "s/@VERSION@/${VERSION}/g" "$ROOT/installer/Info.plist" > "$APP_DIR/Contents/Info.plist"

if [[ -f "$ROOT/assets/island.ico" ]] && command -v sips >/dev/null && command -v iconutil >/dev/null; then
  ICONSET="$STAGE/AppIcon.iconset"
  BASE_PNG="$STAGE/icon_base.png"
  mkdir -p "$ICONSET"
  if sips -s format png "$ROOT/assets/island.ico" --out "$BASE_PNG" >/dev/null 2>&1; then
    sips -z 16 16     "$BASE_PNG" --out "$ICONSET/icon_16x16.png" >/dev/null
    sips -z 32 32     "$BASE_PNG" --out "$ICONSET/icon_16x16@2x.png" >/dev/null
    sips -z 32 32     "$BASE_PNG" --out "$ICONSET/icon_32x32.png" >/dev/null
    sips -z 64 64     "$BASE_PNG" --out "$ICONSET/icon_32x32@2x.png" >/dev/null
    sips -z 128 128   "$BASE_PNG" --out "$ICONSET/icon_128x128.png" >/dev/null
    sips -z 256 256   "$BASE_PNG" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
    sips -z 256 256   "$BASE_PNG" --out "$ICONSET/icon_256x256.png" >/dev/null
    sips -z 512 512   "$BASE_PNG" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
    sips -z 512 512   "$BASE_PNG" --out "$ICONSET/icon_512x512.png" >/dev/null
    sips -z 1024 1024 "$BASE_PNG" --out "$ICONSET/icon_512x512@2x.png" >/dev/null
    iconutil -c icns "$ICONSET" -o "$APP_DIR/Contents/Resources/AppIcon.icns" || true
  fi
fi

SIGN_ID="${ISLAND_MAC_SIGN_IDENTITY:--}"
codesign --force --deep --sign "$SIGN_ID" "$APP_DIR"

ZIP_PATH="$DIST/island-${VERSION}-macos-portable.zip"
rm -f "$ZIP_PATH"
PORTABLE_DIR="$STAGE/portable"
mkdir -p "$PORTABLE_DIR"
ditto "$APP_DIR" "$PORTABLE_DIR/Kite Island.app"
cp "$ROOT/README.md" "$ROOT/README.en.md" "$ROOT/LICENSE" "$ROOT/NOTICE.md" "$PORTABLE_DIR/"
ditto -c -k --sequesterRsrc "$PORTABLE_DIR" "$ZIP_PATH"

DMG_STAGE="$STAGE/dmg"
mkdir -p "$DMG_STAGE"
ditto "$APP_DIR" "$DMG_STAGE/Kite Island.app"
ln -s /Applications "$DMG_STAGE/Applications"
DMG_PATH="$DIST/island-${VERSION}-macos.dmg"
rm -f "$DMG_PATH"
for attempt in 1 2 3; do
  if hdiutil create -volname "Kite Island" -srcfolder "$DMG_STAGE" -ov -format UDZO "$DMG_PATH" >/dev/null; then
    break
  fi
  if [[ "$attempt" -eq 3 ]]; then
    echo "hdiutil create failed after 3 attempts" >&2
    exit 1
  fi
  rm -f "$DMG_PATH"
  sleep 2
done

(
  cd "$DIST"
  shasum -a 256 "island-${VERSION}-macos.dmg" "island-${VERSION}-macos-portable.zip" > SHA256SUMS.txt
)

echo "Built macOS artifacts in $DIST:"
ls -lh "$DMG_PATH" "$ZIP_PATH" "$DIST/SHA256SUMS.txt"
