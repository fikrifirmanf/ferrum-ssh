#!/bin/sh
# Builds dist/Ferrum.app and dist/Ferrum-<version>-macos-<arch>.dmg from a release build.
# Usage: packaging/build-macos.sh [target-triple]   (default: host)
set -eu

cd "$(dirname "$0")/.."
# A release must never silently fall back to an ad-hoc signature.
IDENTITY=${MACOS_SIGNING_IDENTITY:-}
PROFILE=${MACOS_NOTARY_PROFILE:-}
if [ -n "$IDENTITY" ] || [ -n "$PROFILE" ] || [ "${MACOS_REQUIRE_NOTARIZATION:-0}" = 1 ]; then
    case "$IDENTITY" in
        "Developer ID Application: "*) ;;
        *) echo 'Set MACOS_SIGNING_IDENTITY to a Developer ID Application identity.' >&2; exit 1 ;;
    esac
    if [ -z "$PROFILE" ]; then
        echo 'Set MACOS_NOTARY_PROFILE to a notarytool keychain profile.' >&2
        exit 1
    fi
fi
sign() {
    if [ -n "${MACOS_SIGNING_KEYCHAIN:-}" ]; then
        codesign --keychain "$MACOS_SIGNING_KEYCHAIN" "$@"
    else
        codesign "$@"
    fi
}
notarize() {
    SUBMISSION=$1
    set -- --keychain-profile "$PROFILE" --wait
    if [ -n "${MACOS_SIGNING_KEYCHAIN:-}" ]; then
        set -- "$@" --keychain "$MACOS_SIGNING_KEYCHAIN"
    fi
    xcrun notarytool submit "$SUBMISSION" "$@"
}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TARGET=${1:-}
if [ -n "$TARGET" ]; then
    cargo build --release --locked --target "$TARGET"
    BIN=target/$TARGET/release/ferrum
    case "$TARGET" in aarch64*) ARCH=arm64 ;; *) ARCH=x86_64 ;; esac
else
    cargo build --release --locked
    BIN=target/release/ferrum
    ARCH=$(uname -m); [ "$ARCH" = "arm64" ] || ARCH=x86_64
fi

APP=dist/Ferrum.app
rm -rf dist/Ferrum.app dist/iconset.iconset
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" dist/iconset.iconset

cp "$BIN" "$APP/Contents/MacOS/ferrum"
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# .icns from the pre-rendered PNGs
I=packaging/icons
cp $I/icon_16.png   dist/iconset.iconset/icon_16x16.png
cp $I/icon_32.png   dist/iconset.iconset/icon_16x16@2x.png
cp $I/icon_32.png   dist/iconset.iconset/icon_32x32.png
cp $I/icon_64.png   dist/iconset.iconset/icon_32x32@2x.png
cp $I/icon_128.png  dist/iconset.iconset/icon_128x128.png
cp $I/icon_256.png  dist/iconset.iconset/icon_128x128@2x.png
cp $I/icon_256.png  dist/iconset.iconset/icon_256x256.png
cp $I/icon_512.png  dist/iconset.iconset/icon_256x256@2x.png
cp $I/icon_512.png  dist/iconset.iconset/icon_512x512.png
cp $I/icon_1024.png dist/iconset.iconset/icon_512x512@2x.png
iconutil -c icns dist/iconset.iconset -o "$APP/Contents/Resources/Ferrum.icns"
rm -rf dist/iconset.iconset

if [ -n "$IDENTITY" ]; then
    sign --force --timestamp --options runtime --sign "$IDENTITY" "$APP"
else
    echo 'Warning: local ad-hoc build; downloaded copies will trigger Gatekeeper.' >&2
    codesign --force --sign - "$APP"
fi
codesign --verify --deep --strict --verbose=2 "$APP"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
if [ -n "$IDENTITY" ]; then
    # Staple the app before packaging so copied apps also work offline.
    ditto -c -k --keepParent "$APP" "$STAGE/Ferrum.zip"
    notarize "$STAGE/Ferrum.zip"
    xcrun stapler staple "$APP"
    xcrun stapler validate "$APP"
    spctl --assess --type execute --verbose=2 "$APP"
    rm "$STAGE/Ferrum.zip"
fi

DMG=dist/Ferrum-$VERSION-macos-$ARCH.dmg
rm -f "$DMG"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Ferrum" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
if [ -n "$IDENTITY" ]; then
    sign --force --timestamp --sign "$IDENTITY" "$DMG"
    notarize "$DMG"
    xcrun stapler staple "$DMG"
    xcrun stapler validate "$DMG"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
fi
echo "$DMG"
