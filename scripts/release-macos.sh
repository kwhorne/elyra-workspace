#!/usr/bin/env bash
# Build a signed, notarized and stapled release of Elyra Workspace for macOS:
#   target/release/dist/Elyra-Workspace-<version>-<arch>.dmg (+ .sha256)
#
# Needs:
#   CODESIGN_IDENTITY  "Developer ID Application: …" (default: the GETS AS identity)
#   NOTARY_PROFILE     notarytool keychain profile (default: elyra-workspace)
#
# Store the notarization credentials once (an app-specific password from
# appleid.apple.com, or use --key/--key-id/--issuer for an App Store Connect key):
#   xcrun notarytool store-credentials elyra-workspace \
#     --apple-id you@example.com --team-id 7G383N3VY7
#
# In CI, set NOTARY_APPLE_ID, NOTARY_PASSWORD (app-specific) and NOTARY_TEAM_ID
# instead of a keychain profile.
#
# SKIP_NOTARIZE=1 builds and signs without notarizing (local testing only).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="Elyra Workspace"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ARCH=arm64  # Apple Silicon only; bundle-macos.sh refuses other hosts
export CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-Developer ID Application: GETS AS (7G383N3VY7)}"
PROFILE="${NOTARY_PROFILE:-elyra-workspace}"
if [ -n "${NOTARY_APPLE_ID:-}" ]; then
  NOTARY_AUTH=(--apple-id "$NOTARY_APPLE_ID" --password "${NOTARY_PASSWORD:?}" --team-id "${NOTARY_TEAM_ID:-7G383N3VY7}")
else
  NOTARY_AUTH=(--keychain-profile "$PROFILE")
fi
APP="$ROOT/target/release/bundle/$NAME.app"
DIST="$ROOT/target/release/dist"
DMG="$DIST/Elyra-Workspace-$VERSION-$ARCH.dmg"

notarize() {
  if [ "${SKIP_NOTARIZE:-}" = "1" ]; then
    echo "→ skipping notarization of $(basename "$1")"
    return
  fi
  echo "→ notarizing $(basename "$1") (this takes a few minutes)"
  xcrun notarytool submit "$1" "${NOTARY_AUTH[@]}" --wait
}

if [ "${SKIP_NOTARIZE:-}" != "1" ] && ! xcrun notarytool history "${NOTARY_AUTH[@]}" >/dev/null 2>&1; then
  echo "Notarization credentials don't work (profile \"$PROFILE\" or NOTARY_* variables). Store them once with:" >&2
  echo "  xcrun notarytool store-credentials $PROFILE --apple-id <apple id> --team-id 7G383N3VY7" >&2
  exit 1
fi

echo "→ building and signing $NAME $VERSION ($ARCH)"
"$ROOT/scripts/bundle-macos.sh"

mkdir -p "$DIST"
ZIP="$DIST/app.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
notarize "$ZIP"
rm -f "$ZIP"
if [ "${SKIP_NOTARIZE:-}" != "1" ]; then
  xcrun stapler staple "$APP"
fi

echo "→ building the disk image"
STAGE="$DIST/dmg"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
# hdiutil sometimes answers "Resource busy" on CI runners; try a few times.
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "$NAME $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null; then
    break
  fi
  if [ "$attempt" = 5 ]; then
    echo "hdiutil failed five times" >&2
    exit 1
  fi
  echo "hdiutil failed (attempt $attempt); trying again in 10 seconds" >&2
  sleep 10
done
rm -rf "$STAGE"
codesign --force --timestamp --sign "$CODESIGN_IDENTITY" "$DMG"
notarize "$DMG"
if [ "${SKIP_NOTARIZE:-}" != "1" ]; then
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature --verbose "$DMG"
  spctl --assess --type execute --verbose "$APP"
fi

# Checksum after stapling: stapling rewrites the image.
(cd "$DIST" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")
echo "built $DMG"
