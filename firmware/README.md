# Replay 2026 badge firmware

This directory contains the Rust/ESP-IDF Temporal Activity Worker specifically
for the [Temporal Replay 2026 Badge](https://badge.temporal.io/). It is not a
generic ESP32-S3 firmware project. It depends on that badge's 16 MiB flash,
8 MiB PSRAM, OLED, directional buttons, and GPIO mapping.

The Worker renders questions, reads answers, heartbeats while a player decides,
and stores its stable badge/session state in NVS.

Run the commands below from the repository root.

## Hardware and host requirements

- A Temporal Replay 2026 Badge connected with a USB data cable.
- A 2.4 GHz Wi-Fi network that can reach Temporal Cloud.
- macOS or Linux.
- A Temporal Cloud namespace and API key.

The official badge developer guide covers the original MicroPython firmware,
WebSerial, and the complete hardware APIs. This repository replaces the badge
application firmware with a Rust/ESP-IDF image for the trivia Worker.

## Install the ESP Rust toolchain

```sh
cargo install espup --locked
espup install
. "$HOME/export-esp.sh"
cargo install espflash ldproxy
```

Source the `espup` export file in every terminal used to build the firmware.
The workspace pins ESP-IDF v5.4 and selects `xtensa-esp32s3-espidf` through the
root `.cargo/config.toml`.

## Configure the badge

Complete the root [shared Temporal configuration](../README.md#shared-temporal-configuration),
then create the ignored Wi-Fi file:

```sh
cp firmware/.env.wifi.example firmware/.env.wifi
```

Set the badge network in `firmware/.env.wifi`:

```dotenv
BADGE_WIFI_SSID=your-2.4-ghz-network
BADGE_WIFI_PASS=your-password
```

Firmware configuration is compiled into the flash image. Rebuild and reflash
after changing Temporal credentials or Wi-Fi. Generated configuration stays
under the ignored root `target/` directory, and build output does not print
credential values.

Set `BADGE_WIFI_ENV_FILE` to use a Wi-Fi file in another location. An explicit
path must exist.

## Build

```sh
./build-firmware.sh
```

The first build downloads and compiles ESP-IDF dependencies and can take
several minutes. The release ELF is written to:

```text
target/xtensa-esp32s3-espidf/release/temporal-trivia-badge-firmware
```

If the Xtensa compiler is outside a normal `espup` or ESP-IDF installation,
set `ESP_GCC_DIR` to the directory containing `xtensa-esp32s3-elf-gcc`.

## Flash

Connect one Replay 2026 badge and identify its serial device:

```sh
ls /dev/cu.usbmodem* /dev/ttyACM* 2>/dev/null
```

Flash the discovered device, for example:

```sh
./flash-badge.sh /dev/cu.usbmodem101
```

On Linux, use the discovered `/dev/ttyACM...` path. The script selects the
ESP32-S3, 16 MiB flash, `firmware/partitions.csv`, and the factory application
partition. These settings are required because the Rust application is larger
than the default 4 MiB layout.

Flashing this image replaces the badge's installed application firmware. Keep
the official Replay 2026 badge recovery instructions available if you want to
restore the original image later.

The flash script opens a serial monitor. A successful boot reports the stable
badge callsign, Wi-Fi connection, and polling of
`temporal-trivia-badges-v1`. Exit the monitor with `Ctrl+C`; the badge keeps
running. Set `ESPFLASH` to an executable path to override the flashing tool.

## Badge controls

- Press the directional button matching the on-screen answer position.
- Hold **LEFT+RIGHT** for 500 ms to simulate a Worker failure. The badge stops
  heartbeating for six seconds; Temporal's five-second heartbeat timeout makes
  the unfinished question available to another Worker.
- A wrong answer applies the score penalty and completes the Activity normally.
  Only a simulated Worker failure returns the question to the Task Queue.
- While waiting for work, hold **DOWN** for three seconds to sleep. Release it
  after `SLEEPING`, then press any face button to wake. Sleep is disabled while
  an Activity owns the controls.
- Haptics are always on and reserved for meaningful state changes. The sleep
  countdown pulses on `3`, `2`, `1`, and `0`; correct and wrong answers,
  simulated crash/recovery, and round results each have distinct short
  patterns. Routine input, polling, connection changes, boot, and wake are
  silent.

## Firmware verification

The release build is the primary automated firmware gate:

```sh
./build-firmware.sh
```

Physical verification requires checking the serial log and badge display after
flashing. Confirm boot, PSRAM, Wi-Fi, Temporal polling, question rendering,
button input, simulated crash, and sleep/wake behavior.

Confirm the haptic strength and patterns by hand on the physical badge; serial
logs can verify the event path but cannot verify how the motor feels.

See the root [engineering journal](../blog.md) for the current Rust SDK
portability patches and physical validation results.
