#!/bin/sh
set -u

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
host_target=${HOST_TARGET:-$(rustc -vV | awk '/^host:/ { print $2 }')}

while true; do
    TRIVIA_SUPERVISED=1 cargo run \
        --manifest-path "$project_dir/Cargo.toml" \
        -p temporal-trivia-web \
        --bin temporal-trivia-web \
        --target "$host_target"
    status=$?
    if [ "$status" -ne 75 ]; then
        exit "$status"
    fi
    echo "Mac Worker crashed on command; restarting from Temporal history..."
    sleep 2
done
