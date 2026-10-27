#!/bin/sh
# Installs the latest Ferrum release into /Applications on macOS.
# Usage: curl -fsSL https://raw.githubusercontent.com/fikrifirmanf/ferrum-ssh/main/install.sh | sh
# Set FERRUM_INSTALL_DIR to install somewhere else, e.g. ~/Applications.
#
# curl does not set com.apple.quarantine, so the app opens without a Gatekeeper prompt.
set -eu

REPO=fikrifirmanf/ferrum-ssh
DEST=${FERRUM_INSTALL_DIR:-/Applications}

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

# Wrapped in a function so a truncated download over a pipe runs nothing.
main() {
    [ "$(uname -s)" = Darwin ] || fail 'Ferrum installer supports macOS only.'

    # uname -m reports x86_64 under Rosetta, so ask the hardware directly.
    if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
        ARCH=arm64
    else
        ARCH=x86_64
    fi

    echo "Finding the latest Ferrum release for $ARCH..."
    RELEASE=$(curl -fsSL -H 'Accept: application/vnd.github+json' \
        "https://api.github.com/repos/$REPO/releases/latest") \
        || fail 'Could not reach the GitHub releases API.'
    URLS=$(printf '%s\n' "$RELEASE" | sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p')
    DMG_URL=$(printf '%s\n' "$URLS" | grep -E "/Ferrum-[^/]*-macos-$ARCH\.dmg\$" | head -1 || true)
    SUMS_URL=$(printf '%s\n' "$URLS" | grep -E '/SHA256SUMS\.txt$' | head -1 || true)
    [ -n "$DMG_URL" ] || fail "The latest release has no macOS $ARCH DMG."
    DMG_NAME=${DMG_URL##*/}

    WORK=$(mktemp -d)
    MOUNT="$WORK/mount"
    cleanup() {
        hdiutil detach "$MOUNT" -quiet >/dev/null 2>&1 || true
        rm -rf "$WORK"
    }
    trap cleanup EXIT
    trap 'exit 1' INT TERM

    echo "Downloading $DMG_NAME..."
    curl -fL --progress-bar -o "$WORK/$DMG_NAME" "$DMG_URL" || fail 'Download failed.'

    if [ -n "$SUMS_URL" ]; then
        curl -fsSL -o "$WORK/SHA256SUMS.txt" "$SUMS_URL" || fail 'Could not download SHA256SUMS.txt.'
        EXPECTED=$(awk -v f="$DMG_NAME" '$2 == f { print $1 }' "$WORK/SHA256SUMS.txt")
        [ -n "$EXPECTED" ] || fail "SHA256SUMS.txt has no entry for $DMG_NAME."
        ACTUAL=$(shasum -a 256 "$WORK/$DMG_NAME" | awk '{ print $1 }')
        [ "$EXPECTED" = "$ACTUAL" ] || fail "Checksum mismatch for $DMG_NAME."
        echo 'Checksum verified.'
    else
        echo 'Warning: release has no SHA256SUMS.txt; skipping checksum check.' >&2
    fi

    mkdir "$MOUNT"
    hdiutil attach "$WORK/$DMG_NAME" -nobrowse -readonly -noautoopen -mountpoint "$MOUNT" -quiet \
        || fail 'Could not mount the DMG.'
    [ -d "$MOUNT/Ferrum.app" ] || fail 'The DMG does not contain Ferrum.app.'

    if pgrep -xq ferrum; then
        echo 'Quitting the running Ferrum...'
        osascript -e 'quit app "Ferrum"' >/dev/null 2>&1 || true
        sleep 2
    fi

    SUDO=
    if [ ! -w "$DEST" ] || { [ -e "$DEST/Ferrum.app" ] && [ ! -w "$DEST/Ferrum.app" ]; }; then
        echo "Administrator access is needed to write to $DEST."
        SUDO=sudo
    fi
    echo "Installing Ferrum.app into $DEST..."
    $SUDO rm -rf "$DEST/Ferrum.app"
    $SUDO ditto "$MOUNT/Ferrum.app" "$DEST/Ferrum.app"
    # Clears a flag left by an earlier browser download of the same app.
    $SUDO xattr -dr com.apple.quarantine "$DEST/Ferrum.app" 2>/dev/null || true

    echo "Ferrum installed. Open it from $DEST or run: open -a Ferrum"
}

main "$@"
