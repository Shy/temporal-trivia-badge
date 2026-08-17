# Durable Trivia

Durable Trivia is a 60-second competitive game where ESP32-S3 conference
badges are real Rust Temporal Activity Workers. A Rust/Axum controller runs the
Workflow Worker and a 16:9 scoreboard on a laptop; Temporal Cloud coordinates
questions, retries unfinished work, and preserves the round through crashes.

- `firmware/` contains the Rust/ESP-IDF badge Worker, OLED UI, buttons,
  deterministic badge identity, and NVS session state.
- `shared/` contains the serialized game contract used by both Workers, so the
  firmware and controller cannot drift independently.
- `web/` contains the Rust Workflow Worker, operator server, scoreboard, and
  bundled trivia deck.

See [GAME_SPEC.md](GAME_SPEC.md) for the game rules and [blog.md](blog.md) for
the engineering log, including the current Rust SDK portability patches.

## Requirements

- A Temporal Cloud namespace and API key.
- A 2.4 GHz Wi-Fi network reachable by the badges and laptop.
- Rust installed with `rustup`.
- An ESP32-S3 badge with 16 MiB flash and the hardware mapping used by this
  repository.
- macOS or Linux for the controller. The provided flashing commands use Unix
  device paths.

Install the ESP Rust toolchain and flashing tools:

```sh
cargo install espup --locked
espup install
. "$HOME/export-esp.sh"
cargo install espflash ldproxy
```

The `espup` export file must be sourced in every terminal used to build the
firmware. The project pins ESP-IDF v5.4 and selects the
`xtensa-esp32s3-espidf` target through `.cargo/config.toml`.

## Configure Temporal Cloud and Wi-Fi

Create the two ignored configuration files:

```sh
cp .env.temporal.example .env.temporal
cp firmware/.env.wifi.example firmware/.env.wifi
```

Fill in `.env.temporal`:

```dotenv
TEMPORAL_ADDRESS=your-namespace.tmprl.cloud:7233
TEMPORAL_NAMESPACE=your-namespace.your-account
TEMPORAL_API_KEY=your-api-key
```

Fill in `firmware/.env.wifi`:

```dotenv
BADGE_WIFI_SSID=your-2.4-ghz-network
BADGE_WIFI_PASS=your-password
```

The controller also accepts the three `TEMPORAL_*` values as process
environment variables; those override `.env.temporal`. Firmware configuration
is compiled into the image, so rebuild and reflash after changing credentials
or Wi-Fi. Never commit either populated file.

Set `TEMPORAL_ENV_FILE` or `BADGE_WIFI_ENV_FILE` to use config files in another
location. An explicit path must exist; the build fails immediately if it is
mistyped. The generated firmware config stays under the ignored `target/`
directory, and build output does not print credential values.

Temporal Cloud always uses server-authenticated TLS. The API key replaces a
client certificate; it does not disable TLS.

## Build and flash a badge

Build the release firmware from the repository root:

```sh
./build-firmware.sh
```

The first build downloads and compiles ESP-IDF dependencies and can take
several minutes. The output ELF is:

```text
target/xtensa-esp32s3-espidf/release/temporal-trivia-badge-firmware
```

Connect one badge, find its serial port, and flash it:

```sh
ls /dev/cu.usbmodem* /dev/ttyACM* 2>/dev/null
./flash-badge.sh /dev/cu.usbmodem101
```

On Linux, substitute the discovered `/dev/ttyACM...` path. The flash script
selects ESP32-S3, 16 MiB flash, the repository partition table, and the factory
application partition; those options are required for this firmware. It opens
a serial monitor after flashing. Exit the monitor with `Ctrl+C`; the badge
continues running.

If the Xtensa compiler is installed outside the normal espup/ESP-IDF location,
set `ESP_GCC_DIR` to the directory containing
`xtensa-esp32s3-elf-gcc`. Set `ESPFLASH` to an executable path to override the
flashing tool.

## Run the controller and scoreboard

Start the controller under the included restart supervisor:

```sh
./run-web.sh
```

Open <http://127.0.0.1:3000> and mirror that browser window to the TV. The
script detects the host Rust target, marks the process as supervised, and
restarts it after the operator deliberately crashes the Mac Worker. Temporal
history restores the active round after restart.

Click **START ROUND** after badges show as connected. Only one round can run at
a time. Badges that begin polling during a round join automatically.

The operator drawer can send durable Workflow Signals for double points,
Rust-only scheduling, sudden death, or one 30-second extension. Completed
rounds are stored in Workflow Memo and listed through Temporal Visibility; no
database or namespace changes are required.

Optional typed Search Attributes can be registered by an API key with
namespace-operator permission:

```sh
./configure-visibility.sh
TRIVIA_SEARCH_ATTRIBUTES=1 ./run-web.sh
```

The game and round history still work when those attributes are not registered.

## Badge controls

- Press the directional button matching the on-screen answer position.
- Hold LEFT+RIGHT for 500 ms to simulate a Worker failure. The badge stops
  heartbeating for six seconds; Temporal's five-second heartbeat timeout makes
  the unfinished question available to another Worker.
- A wrong answer applies the score penalty and fails the Activity retryably, so
  the question returns to the Task Queue.
- While waiting for work, hold DOWN for three seconds to sleep. Release DOWN
  after `SLEEPING`, then press any face button to wake. Sleep is disabled while
  an Activity owns the controls.

## Verification

Run the host-side Workflow and question-pool tests:

```sh
host_target=$(rustc -vV | awk '/^host:/ { print $2 }')
cargo test --offline -p temporal-trivia-shared -p temporal-trivia-web --target "$host_target"
cargo clippy --offline -p temporal-trivia-shared -p temporal-trivia-web \
  --target "$host_target" --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

The most recent physical validation used one ESP32-S3 badge. Build, flash,
boot, Wi-Fi, Temporal Cloud polling, answers, sleep/wake, and supervised Mac
Worker recovery were exercised. A timeout visibly moving between two physical
badges remains the outstanding multi-device validation.
