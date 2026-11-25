#!/bin/sh
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/Milo.app"
DMG="$ROOT/target/Milo.dmg"
STAGE="$ROOT/target/dmg"
MANIFEST="$ROOT/crates/milo/Cargo.toml"
REPO="govindpuff/milo"
PROFILE="${MILO_NOTARY:-milo}"

if [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    cat <<TEXT
Usage: scripts/release.sh [VERSION]

Releases Milo: builds Milo.app, has Apple notarize it, makes target/Milo.dmg,
and publishes it on GitHub as the release vVERSION.

With VERSION, such as 0.2.0, the script first sets that version in
crates/milo/Cargo.toml and commits the change. Without it, the script
releases the version that is already there.

This needs a Developer ID Application certificate in your keychain, the gh
command signed in to GitHub, and notarization credentials saved one time
under the name milo:

  xcrun notarytool store-credentials milo --apple-id YOUR_APPLE_ID --team-id YOUR_TEAM_ID

To use credentials saved under another name, set MILO_NOTARY.
TEXT
    exit 0
fi

fail() {
    echo "$*" >&2
    exit 1
}

git() {
    command git -C "$ROOT" "$@"
}

[ "$(git branch --show-current)" = main ] || fail "Releases are cut from main. Switch to the main branch first."
[ -z "$(git status --porcelain)" ] || fail "There are changes that aren't committed. Commit or remove them first."
gh auth status >/dev/null 2>&1 || fail "The gh command isn't signed in. Run: gh auth login"

WAS="$(sed -n 's/^version = "\(.*\)"/\1/p' "$MANIFEST" | head -1)"
VERSION="${1:-$WAS}"
if gh release view "v$VERSION" --repo "$REPO" >/dev/null 2>&1; then
    fail "Milo $VERSION is already released. Give a newer version: scripts/release.sh VERSION"
fi

if [ "$VERSION" != "$WAS" ]; then
    sed -i '' "1,/^version = /s/^version = .*/version = \"$VERSION\"/" "$MANIFEST"
    cargo update --quiet --workspace --manifest-path "$ROOT/Cargo.toml"
    git add "$MANIFEST" "$ROOT/Cargo.lock"
    git commit --quiet -m "Raise the version to $VERSION"
fi

"$ROOT/scripts/bundle.sh"
SIGN="$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=\(Developer ID Application: .*\)/\1/p' | head -1)"
[ -n "$SIGN" ] || fail "There is no Developer ID Application certificate in your keychain, so the app can't be notarized."

ditto -c -k --keepParent "$APP" "$ROOT/target/Milo.zip"
xcrun notarytool submit "$ROOT/target/Milo.zip" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$APP"

rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/Milo.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname Milo -srcfolder "$STAGE" -format UDZO "$DMG"
codesign --timestamp --sign "$SIGN" "$DMG"
xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$DMG"

git push --quiet origin main
gh release create "v$VERSION" "$DMG" --repo "$REPO" --target main --title "Milo $VERSION" --generate-notes
