#!/bin/bash
set -euo pipefail

APP_NAME="I'll Allow It"
BUNDLE_DIR="$APP_NAME.app"
BINARY_NAME="ill-allow-it"

echo "Building release binary..."
cargo build --release

echo "Creating app bundle..."
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS"
mkdir -p "$BUNDLE_DIR/Contents/Resources"

cp "target/release/$BINARY_NAME" "$BUNDLE_DIR/Contents/MacOS/"
cp Info.plist "$BUNDLE_DIR/Contents/"

echo "App bundle created at: $BUNDLE_DIR"
echo ""
echo "To install:"
echo "  cp -r \"$BUNDLE_DIR\" /Applications/"
echo ""
echo "Then grant Accessibility permission:"
echo "  System Settings > Privacy & Security > Accessibility > add \"$APP_NAME\""
