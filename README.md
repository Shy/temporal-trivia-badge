# Temporal Trivia Badge

A 60-second competitive trivia game where ESP32-S3 conference badges are real
Rust Temporal Activity Workers. A Rust/Axum controller on a MacBook starts the
game and drives the TV display; Temporal Cloud owns the durable game state.

The repository has two project folders:

- `firmware/` — Rust/ESP-IDF badge Worker, OLED UI, buttons, identity, and NVS.
- `web/` — Rust Temporal Workflow Worker, game controller, TV UI, and questions.

See [GAME_SPEC.md](GAME_SPEC.md) for the locked game contract.

## Local configuration

1. Copy `firmware/.env.wifi.example` to `firmware/.env.wifi` and fill it in.
2. Temporal Cloud settings are loaded from the existing ignored
   `/Users/shy/Documents/Temporal/TrafficLight/.env` file at build/run time.
3. Run `./build-firmware.sh`.
4. Connect a badge and run `./flash-badge.sh /dev/cu.usbmodem...`. The script
   passes the badge's 16 MiB partition layout explicitly and opens a serial
   monitor after flashing.

The web controller runs with:

```sh
cargo run -p temporal-trivia-web --target aarch64-apple-darwin
```

Then open <http://127.0.0.1:3000> on the mirrored MacBook display.

The firmware retains server certificate and hostname verification. API-key
authentication does not remove the server-TLS requirement for Temporal Cloud.
