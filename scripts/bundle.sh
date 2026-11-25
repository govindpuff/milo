#!/bin/sh
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/Milo.app"
UV_VERSION="0.11.19"
UV_SHA="d8f59c38e8c4168ee468d423cd63184be12fa6995a4283d41ee1a14d003c9453"
UV="$ROOT/target/uv-$UV_VERSION"

if [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    cat <<TEXT
Usage: scripts/bundle.sh

Builds milo and puts it in target/Milo.app, with the uv command that installs
the voice.

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
if [ ! -x "$UV/uv" ]; then
    mkdir -p "$UV"
    curl -fsSL "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/uv-aarch64-apple-darwin.tar.gz" -o "$UV.tar.gz"
    echo "$UV_SHA  $UV.tar.gz" | shasum -a 256 -c - >/dev/null
    tar -xzf "$UV.tar.gz" -C "$UV" --strip-components 1
fi
cp "$UV/uv" "$APP/Contents/MacOS/uv"
cp "$ROOT/app/Milo.icns" "$APP/Contents/Resources/Milo.icns"
sed "s/VERSION/$VERSION/" "$ROOT/app/Info.plist" > "$APP/Contents/Info.plist"

SIGN="${MILO_SIGN:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)}"
STAMP="--timestamp"
if [ -z "$SIGN" ]; then
    SIGN="-"
    STAMP=""
fi
codesign --force $STAMP --options runtime --sign "$SIGN" "$APP/Contents/MacOS/uv"
codesign --force $STAMP --options runtime --entitlements "$ROOT/app/milo.entitlements" --sign "$SIGN" "$APP"

echo "Built $APP, signed as $SIGN"
