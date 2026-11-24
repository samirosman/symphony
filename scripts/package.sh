#!/usr/bin/env bash
# Package a release build for one target (ENGINEERING E13.3).
# Usage: scripts/package.sh <version> <target>   (run after cargo build --release --target <target>)
# Produces in dist/:
#   symphony-<version>-<target>.tar.gz|zip     the app, read by the launcher
#   symphony-launcher-<target>.zip|tar.gz      what testers download
set -euo pipefail
VERSION="$1"
TARGET="$2"
BIN="target/$TARGET/release"
EXE=""
[[ "$TARGET" == *windows* ]] && EXE=".exe"
mkdir -p dist
STAGE="$(mktemp -d)"

# App archive: executable at the top level, plus licence and readme.
APP="$STAGE/app"
mkdir -p "$APP"
cp "$BIN/symphony$EXE" "$APP/"
cp LICENSE README.md "$APP/"
WORDMARK_LICENSE="crates/symphony-ui/assets/bricolage/OFL.txt"
cp "$WORDMARK_LICENSE" "$APP/OFL-Bricolage-Grotesque.txt"
cp "$WORDMARK_LICENSE" "$STAGE/OFL-Bricolage-Grotesque.txt"
NAME="symphony-$VERSION-$TARGET"
if [[ "$TARGET" == *windows* ]]; then
  (cd "$APP" && 7z a -tzip "$OLDPWD/dist/$NAME.zip" . > /dev/null)
else
  chmod +x "$APP/symphony"
  tar -C "$APP" -czf "dist/$NAME.tar.gz" .
fi

# Launcher. On macOS there's one universal launcher (both chips), packaged
# by the Apple-chip job as symphony-launcher-macos.zip; the Intel job only
# packages the app.
L="symphony-launcher-$TARGET"
if [[ "$TARGET" == x86_64-apple-darwin ]]; then
  echo "Intel Mac: the universal launcher is packaged by the Apple-chip build."
elif [[ "$TARGET" == *apple* ]]; then
  L="symphony-launcher-macos"
  LBIN="target/universal-apple-darwin/release/symphony-launcher"
  [[ -f "$LBIN" ]] || LBIN="$BIN/symphony-launcher"
  BUNDLE="$STAGE/Symphony Launcher.app"
  mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
  cp "$WORDMARK_LICENSE" "$BUNDLE/Contents/Resources/OFL-Bricolage-Grotesque.txt"
  cp "$LBIN" "$BUNDLE/Contents/MacOS/symphony-launcher"
  cp crates/symphony-app/assets/Symphony.icns "$BUNDLE/Contents/Resources/"
  cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Symphony Launcher</string>
  <key>CFBundleDisplayName</key><string>Symphony Launcher</string>
  <key>CFBundleIdentifier</key><string>io.github.tdemers218.symphony.launcher</string>
  <key>CFBundleExecutable</key><string>symphony-launcher</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>Symphony.icns</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
  # Ad-hoc sign the whole bundle (no Developer ID: testers trust it once
  # in Privacy & Security). An unsigned bundle downloaded in a browser can
  # be refused as "damaged" with no way to open it.
  codesign --force --deep --sign - "$BUNDLE"
  codesign --verify --deep --strict "$BUNDLE"
  ditto -c -k --norsrc --noextattr --keepParent "$BUNDLE" "dist/$L.zip"
elif [[ "$TARGET" == *windows* ]]; then
  cp "$BIN/symphony-launcher.exe" "$STAGE/"
  (cd "$STAGE" && 7z a -tzip "$OLDPWD/dist/$L.zip" symphony-launcher.exe OFL-Bricolage-Grotesque.txt > /dev/null)
else
  cp "$BIN/symphony-launcher" "$STAGE/"
  chmod +x "$STAGE/symphony-launcher"
  tar -C "$STAGE" -czf "dist/$L.tar.gz" symphony-launcher OFL-Bricolage-Grotesque.txt
fi
rm -rf "$STAGE"
ls -l dist
