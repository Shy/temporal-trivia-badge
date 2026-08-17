#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
generated_defaults="$project_dir/.sdkconfig.partition.defaults"
next_defaults="$generated_defaults.next"

if [ -n "${ESP_GCC_DIR:-}" ]; then
    PATH="$ESP_GCC_DIR:$PATH"
fi
if ! command -v xtensa-esp32s3-elf-gcc >/dev/null 2>&1; then
    tools_root=${IDF_TOOLS_PATH:-$HOME/.espressif}
    for candidate in "$tools_root"/tools/xtensa-esp-elf/*/xtensa-esp-elf/bin; do
        if [ -x "$candidate/xtensa-esp32s3-elf-gcc" ]; then
            PATH="$candidate:$PATH"
            break
        fi
    done
fi
if ! command -v xtensa-esp32s3-elf-gcc >/dev/null 2>&1; then
    echo "missing xtensa-esp32s3-elf-gcc; run espup install and source its export file" >&2
    exit 1
fi
if [ -x "$project_dir/.tools/bin/ldproxy" ]; then
    PATH="$project_dir/.tools/bin:$PATH"
fi
if ! command -v ldproxy >/dev/null 2>&1; then
    echo "missing ldproxy; install it with: cargo install ldproxy" >&2
    exit 1
fi
export PATH

printf 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="%s/firmware/partitions.csv"\n' \
    "$project_dir" > "$next_defaults"
if ! cmp -s "$next_defaults" "$generated_defaults"; then
    mv "$next_defaults" "$generated_defaults"
else
    rm "$next_defaults"
fi

export ESP_IDF_SDKCONFIG_DEFAULTS="$project_dir/firmware/sdkconfig.defaults;$generated_defaults"
export ESP_IDF_SYS_ROOT_CRATE="temporal-trivia-badge-firmware"
export BADGE_BUILD_UNIX_EPOCH=$(date +%s)

cd "$project_dir"
exec cargo build -j 2 -p temporal-trivia-badge-firmware --release "$@"
