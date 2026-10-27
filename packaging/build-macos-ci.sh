#!/bin/sh
# Import CI credentials into a disposable keychain, then package Ferrum.
set -eu
set +x
cd "$(dirname "$0")/.."

if [ -n "${CIRCLE_TAG:-}" ] || [ -n "${MACOS_CERTIFICATE_P12_BASE64:-}" ]; then
    export MACOS_REQUIRE_NOTARIZATION=1
fi
if [ -z "${MACOS_CERTIFICATE_P12_BASE64:-}" ]; then
    exec sh packaging/build-macos.sh "$@"
fi

: "${MACOS_CERTIFICATE_PASSWORD:?Set the exported P12 password}"
: "${MACOS_SIGNING_IDENTITY:?Set the Developer ID Application identity}"
: "${APPLE_ID:?Set the Apple developer account email}"
: "${APPLE_TEAM_ID:?Set the Apple developer team ID}"
: "${APPLE_APP_SPECIFIC_PASSWORD:?Set an Apple app-specific password}"

umask 077
WORK=$(mktemp -d)
KEYCHAIN="$WORK/signing.keychain-db"
cleanup() {
    security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
    rm -rf "$WORK"
}
trap cleanup EXIT
KEYCHAIN_PASSWORD=$(openssl rand -hex 32)
printf '%s' "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$WORK/certificate.p12"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security import "$WORK/certificate.p12" -k "$KEYCHAIN" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
export MACOS_SIGNING_KEYCHAIN="$KEYCHAIN"
export MACOS_NOTARY_PROFILE=ferrum-ci
xcrun notarytool store-credentials "$MACOS_NOTARY_PROFILE" --keychain "$KEYCHAIN" \
    --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_SPECIFIC_PASSWORD" >/dev/null
sh packaging/build-macos.sh "$@"
