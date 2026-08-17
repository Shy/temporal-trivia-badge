#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
generated_defaults="$project_dir/.sdkconfig.partition.defaults"
next_defaults="$generated_defaults.next"
esp_gcc_dir="/Users/shy/.espressif/tools/xtensa-esp-elf/esp-14.2.0_20241119/xtensa-esp-elf/bin"

if [ ! -x "$esp_gcc_dir/xtensa-esp32s3-elf-gcc" ]; then
    echo "missing Espressif compiler: $esp_gcc_dir/xtensa-esp32s3-elf-gcc" >&2
    exit 1
fi
export PATH="$project_dir/.tools/bin:$esp_gcc_dir:$PATH"

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
