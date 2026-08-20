#!/bin/sh
set -eu

cd "$(dirname "$0")"
host_target=${HOST_TARGET:-$(rustc -vV | awk '/^host:/ { print $2 }')}
badge_count=${1:-10}
case "$badge_count" in
    ''|*[!0-9]*) echo "badge count must be an integer" >&2; exit 2 ;;
esac
if [ "$badge_count" -lt 1 ] || [ "$badge_count" -gt 100 ]; then
    echo "badge count must be between 1 and 100" >&2
    exit 2
fi

cargo build -p temporal-trivia-web --bin simulate-badges --target "$host_target"
binary="target/$host_target/debug/simulate-badges"
pids=""
cleanup() {
    if [ -n "$pids" ]; then
        kill $pids 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

run_badge() {
    badge_index=$1
    while true; do
        if "$binary" "$badge_index"; then
            return 0
        fi
        echo "SIM-$(printf '%02d' "$badge_index") disconnected; retrying in 2 seconds" >&2
        sleep 2
    done
}

index=1
while [ "$index" -le "$badge_count" ]; do
    run_badge "$index" &
    pids="$pids $!"
    index=$((index + 1))
done
echo "$badge_count simulated badge processes started; stop with Ctrl-C"
wait
