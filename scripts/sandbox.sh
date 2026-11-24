#!/bin/sh
set -e

VM="${MILO_SANDBOX:-milo-sandbox}"
KEY="$HOME/.lume/keys/sandbox"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SSH="-i $KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"

address() {
    lume ls | awk -v vm="$VM" '$1 == vm { for (i = 1; i <= NF; i++) if ($i ~ /^192\./) print $i }'
}

guest() {
    ssh $SSH "lume@$(address)" "$@"
}

usage() {
    cat <<TEXT
Usage: scripts/sandbox.sh COMMAND

Runs milo's computer use inside a macOS virtual machine, so that it never
touches this Mac's desktop. See docs/architecture.md for the one-time setup.

  up                    Start the virtual machine and connect it to the model server on port 8000.
  push                  Build milo and copy it into the virtual machine.
  task TEXT [STEP...]   Carry out one task there, and print each action and the result.
  pull DIRECTORY        Copy the pictures and lists of the last task into DIRECTORY.
  shot FILE             Save a picture of the virtual machine's screen as FILE.
  run COMMAND           Run a command in the virtual machine's desktop session.
  down                  Stop the virtual machine.
TEXT
}

case "${1:-}" in
up)
    lume ls | grep -q "^$VM .* running" || lume run "$VM" --display none --detach
    until [ -n "$(address)" ] && guest true 2>/dev/null; do sleep 2; done
    pkill -f "ssh .*-R 8000:127.0.0.1:8000" 2>/dev/null || true
    ssh -f -N $SSH -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -R 8000:127.0.0.1:8000 "lume@$(address)"
    guest "~/gui.sh osascript -e 'set volume with output muted'"
    echo "ready at $(address)"
    ;;
push)
    cargo build --release --manifest-path "$ROOT/Cargo.toml" -p milo
    scp -q $SSH "$ROOT/target/release/milo" "lume@$(address):"
    ;;
task)
    shift
    text="$1"
    shift
    steps=""
    for step in "$@"; do steps="$steps --step '$step'"; done
    guest "~/gui.sh ~/milo --task '$text' $steps 2>&1"
    ;;
pull)
    mkdir -p "$2"
    where=$(guest 'find /var/folders -type d -name milo-computer 2>/dev/null | head -1')
    scp -q -r $SSH "lume@$(address):$where/*" "$2/"
    ;;
shot)
    guest "~/gui.sh screencapture -x -t jpg /tmp/screen.jpg"
    scp -q $SSH "lume@$(address):/tmp/screen.jpg" "$2"
    ;;
run)
    shift
    guest "~/gui.sh $*"
    ;;
down)
    pkill -f "ssh .*-R 8000:127.0.0.1:8000" 2>/dev/null || true
    lume stop "$VM"
    ;;
*)
    usage
    ;;
esac
