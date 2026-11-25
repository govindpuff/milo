#!/bin/bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./setup.sh

Installs what milo needs on an Apple Silicon Mac and builds the milo command.
Run it again at any time; it skips what is already installed.

  --help    show this help
EOF
}

say() { printf '%s\n' "$*"; }
fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

for arg in "$@"; do
  case "$arg" in
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  fail "Sorry, milo runs only on Apple Silicon Macs (macOS on arm64)."
fi

repo=$(cd "$(dirname "$0")" && pwd)
root="$HOME/.cache/milo"
venv="$root/venv"
cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
user_path="$PATH"
export PATH="$cargo_bin:$HOME/.local/bin:$PATH"

if ! xcode-select -p >/dev/null 2>&1; then
  say "Installing the Xcode Command Line Tools"
  xcode-select --install || true
  fail "Finish the Command Line Tools install in the window that opened, then run ./setup.sh again."
fi

if ! command -v cargo >/dev/null 2>&1; then
  say "Installing Rust"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi

if ! command -v uv >/dev/null 2>&1; then
  say "Installing uv"
  curl -LsSf https://astral.sh/uv/install.sh | sh
fi

if ! command -v imsg >/dev/null 2>&1 && command -v brew >/dev/null 2>&1; then
  say "Installing imsg, which sends text messages"
  brew install steipete/tap/imsg || true
fi

say "Setting up the voice environment in ~/.cache/milo/venv"
mkdir -p "$root"
if [ ! -x "$venv/bin/python" ]; then
  uv venv --python 3.12 "$venv"
fi
uv pip install --python "$venv/bin/python" -r "$repo/sidecar/requirements.txt"

say "Building and installing milo"
cargo install --locked --path "$repo/crates/milo"

say ""
case ":$user_path:" in
  *":$cargo_bin:"*) ;;
  *) say "Note: $cargo_bin is not on your PATH yet. Open a new terminal, or add it to PATH." ;;
esac
say "Done. Save your OpenAI API key with: milo auth openai"
say "Then start it with: milo"
say "The first start downloads the speech and voice models, and macOS asks for microphone access."
say "More options: milo --help"
