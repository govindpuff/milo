#!/bin/sh
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/Milo.app"

if [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    cat <<TEXT
Usage: scripts/bundle.sh

Builds milo and puts it in target/Milo.app.

The app is signed with the Developer ID Application certificate in your
keychain. Without one, it is signed for this Mac only. To choose a
certificate, set MILO_SIGN to its name.
TEXT
    exit 0
fi

cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" -p milo
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/crates/milo/Cargo.toml" | head -1)"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/milo" "$APP/Contents/MacOS/milo"
cp "$ROOT/app/Milo.icns" "$APP/Contents/Resources/Milo.icns"
sed "s/VERSION/$VERSION/" "$ROOT/app/Info.plist" > "$APP/Contents/Info.plist"

SIGN="${MILO_SIGN:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)}"
codesign --force --options runtime --entitlements "$ROOT/app/milo.entitlements" --sign "${SIGN:--}" "$APP"

echo "Built $APP, signed with ${SIGN:-an ad hoc signature}"
