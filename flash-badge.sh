#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
espflash="$project_dir/.tools/bin/espflash"
firmware_elf="$project_dir/target/xtensa-esp32s3-espidf/release/temporal-trivia-badge-firmware"
partition_table="$project_dir/firmware/partitions.csv"

if [ ! -x "$espflash" ]; then
    echo "missing espflash: $espflash" >&2
    exit 1
fi

if [ ! -f "$firmware_elf" ]; then
    echo "missing firmware build; run ./build-firmware.sh first" >&2
    exit 1
fi

port_args=""
if [ "$#" -gt 1 ]; then
    echo "usage: $0 [serial-port]" >&2
    exit 1
fi
if [ "$#" -eq 1 ]; then
    port_args="--port $1"
fi

# The explicit partition table is required. Without it, espflash assumes a
# 4 MiB default app partition even though the ELF was built for this 16 MiB
# badge layout.
# shellcheck disable=SC2086
exec "$espflash" flash \
    --chip esp32s3 \
    --flash-size 16mb \
    --partition-table "$partition_table" \
    --target-app-partition factory \
    --monitor \
    $port_args \
    "$firmware_elf"
