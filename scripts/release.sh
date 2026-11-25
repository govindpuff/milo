#!/bin/sh
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/Milo.app"
DMG="$ROOT/target/Milo.dmg"
STAGE="$ROOT/target/dmg"
PROFILE="${MILO_NOTARY:-milo}"
KEYCHAIN="${MILO_KEYCHAIN:+--keychain $MILO_KEYCHAIN}"

if [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    cat <<TEXT
Usage: scripts/release.sh

Builds Milo.app, has Apple notarize it, and puts it in target/Milo.dmg.

This needs a Developer ID Application certificate in your keychain, and
notarization credentials saved one time under the name milo:

  xcrun notarytool store-credentials milo --apple-id YOUR_APPLE_ID --team-id YOUR_TEAM_ID

To use credentials saved under another name, set MILO_NOTARY. To read them
from a keychain other than your login keychain, set MILO_KEYCHAIN to its path.
TEXT
    exit 0
fi

"$ROOT/scripts/bundle.sh"
SIGN="$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=\(Developer ID Application: .*\)/\1/p' | head -1)"
if [ -z "$SIGN" ]; then
    echo "There is no Developer ID Application certificate in your keychain, so the app can't be notarized." >&2
    exit 1
fi
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/crates/milo/Cargo.toml" | head -1)"

ditto -c -k --keepParent "$APP" "$ROOT/target/Milo.zip"
xcrun notarytool submit "$ROOT/target/Milo.zip" --keychain-profile "$PROFILE" $KEYCHAIN --wait
xcrun stapler staple "$APP"

rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/Milo.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname Milo -srcfolder "$STAGE" -format UDZO "$DMG"
codesign --timestamp --sign "$SIGN" "$DMG"
xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" $KEYCHAIN --wait
xcrun stapler staple "$DMG"

echo "Built $DMG"
echo "To publish it: gh release create v$VERSION \"$DMG\" --title \"Milo $VERSION\" --generate-notes"
