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
Usage: scripts/release.sh patch|minor|major

Releases Milo: builds Milo.app, has Apple notarize it, makes target/Milo.dmg,
and publishes it on GitHub.

The new version is the latest release with one part raised: patch takes
0.1.0 to 0.1.1, minor takes it to 0.2.0, and major takes it to 1.0.0. The
script sets that version in crates/milo/Cargo.toml and commits the change.

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
LATEST="$(gh release view --repo "$REPO" --json tagName --jq .tagName 2>/dev/null | sed 's/^v//')"
LATEST="${LATEST:-0.0.0}"
MAJOR="$(echo "$LATEST" | cut -d. -f1)"
MINOR="$(echo "$LATEST" | cut -d. -f2)"
PATCH="$(echo "$LATEST" | cut -d. -f3)"
case "$1" in
patch) VERSION="$MAJOR.$MINOR.$((PATCH + 1))" ;;
minor) VERSION="$MAJOR.$((MINOR + 1)).0" ;;
major) VERSION="$((MAJOR + 1)).0.0" ;;
*) fail "Say which part of the version to raise: scripts/release.sh patch, minor, or major. The latest release is $LATEST." ;;
esac
echo "Releasing Milo $VERSION (the latest release is $LATEST)"

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
