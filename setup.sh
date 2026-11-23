#!/bin/bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./setup.sh [--lure]

Installs what milo needs on an Apple Silicon Mac and builds the milo command.
Run it again at any time; it skips what is already installed.

  --lure    also install the Lure voice (milo --tts lure)
  --help    show this help
EOF
}

say() { printf '%s\n' "$*"; }
fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

lure=0
for arg in "$@"; do
  case "$arg" in
    --lure) lure=1 ;;
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
lure_commit=3f4ae026d0b0278886d1cd6f386960947d591d7b
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

if ! command -v rapid-mlx >/dev/null 2>&1; then
  say "Installing Rapid-MLX"
  uv tool install rapid-mlx==1.1.0
fi

say "Setting up the voice environment in ~/.cache/milo/venv"
mkdir -p "$root"
if [ ! -x "$venv/bin/python" ]; then
  uv venv --python 3.12 "$venv"
fi
uv pip install --python "$venv/bin/python" -r "$repo/sidecar/requirements.txt"

if [ "$lure" = 1 ]; then
  say "Installing Lure in ~/.cache/milo/lure-tts"
  if [ ! -d "$root/lure-tts" ]; then
    git clone https://github.com/smirki/lure-tts "$root/lure-tts"
  fi
  git -C "$root/lure-tts" checkout --quiet "$lure_commit"
  (cd "$root/lure-tts" && uv pip install --python "$venv/bin/python" ./backends/mlx ".[server]")
fi

say "Building and installing milo"
cargo install --locked --path "$repo/crates/milo"

say ""
case ":$user_path:" in
  *":$cargo_bin:"*) ;;
  *) say "Note: $cargo_bin is not on your PATH yet. Open a new terminal, or add it to PATH." ;;
esac
say "Done. Start it with: milo"
say "The first start downloads the models (several GB), and macOS asks for microphone access."
say "More options: milo --help"
