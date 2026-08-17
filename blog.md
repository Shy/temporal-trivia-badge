# Engineering journal

## 2026-08-17 — First playable Temporal trivia build

- Created a standalone Git repository with two product folders: `firmware/`
  for the ESP32-S3 Rust Worker and `web/` for the Rust Workflow Worker,
  controller API, SSE feed, TV UI, and committed question data.
- Kept the Activity Worker boundary already proven on this badge. The firmware
  uses the locally patched Temporal Rust SDK 0.5.0 needed for ESP-IDF, connects
  to Temporal Cloud with API-key authentication and verified server TLS, and
  derives a stable callsign from the factory MAC.
- Implemented the OLED question UI with word wrapping and the original badge's
  positional Nintendo-style glyphs. Implemented TOP, RIGHT, LEFT, and DOWN
  answer input plus a 500 ms LEFT+RIGHT simulated crash, three-second recovery,
  retryable Activity failure, and NVS-backed question refusal.
- Limited each physical Worker to one concurrent Activity. The SDK tuner
  otherwise defaults to enough Activity slots for multiple questions to race
  over the badge's single display and button cluster.
- Implemented the durable 60-second Workflow, dynamic
  `max(10, active_badges * 2)` backlog, scoring, tied winners, latest-answer
  spotlight, and single-game controller guard. The HTTP start path reserves the
  game before its Cloud call so simultaneous clicks cannot start two games.
- Committed an Open Trivia DB snapshot from
  `leakyhose/open-trivia-script-data`, attributed under CC BY-SA 4.0, and mixed
  it with authored Rust, Temporal, and generated math questions. Tests enforce
  the first 100 as 30 Rust, 15 Temporal, 15 math, and 40 general questions,
  reject display overflow, and reject duplicates.
- Host tests passed: 5 tests, 0 failures. The Rust web controller connected to
  the configured Temporal Cloud namespace and served both `/api/state` and the
  TV page at `127.0.0.1:3000`. Browser inspection found no page overflow at a
  1280x720 viewport.
- The first live controller connection failed with `Connecting to HTTPS without
  TLS enabled`. The endpoint was HTTPS, but the Mac client omitted
  `TlsOptions`; adding default server TLS fixed the connection. API-key auth
  does not remove Temporal Cloud's TLS requirement.
- The first real smoke Workflow then failed every Workflow task with
  `[TMPRL1100] Nondeterministic future detected` because the implementation
  uses `FuturesUnordered` to maintain the dynamic Activity backlog. SDK Core's
  own `wait_condition_waker_in_futures_unordered` test documents that its
  forwarding wakers fall outside the detector guard and disables the detector
  for that case. Applied the same narrow Worker-level opt-out; every future in
  this Workflow remains an SDK Activity or SDK timer.
- The firmware release build passed in 2m05s. An initial `espflash save-image`
  check incorrectly reported the 8 MB image against a 4 MB app partition
  because the command omitted the custom table. Passing `firmware/partitions.csv`
  confirmed 8,000,128 / 14,680,064 bytes (54.50%). `flash-badge.sh` now always
  passes the explicit 16 MB badge layout.
- Live flashing is still pending. macOS currently exposes only Bluetooth,
  debug-console, and earbud serial ports; no Espressif `/dev/cu.usbmodem*`
  device is present. No flash attempt was made against an unresolved target.
- After disabling the detector for this documented combinator case, the
  previously stuck smoke Workflow replayed and completed. A fresh fixed-ID
  Cloud round scheduled 10 unique Activities, populated the 60-second global
  deadline, rejected a concurrent start with HTTP 409, and completed with
  `Round finished with no answers` at the deadline.
- Replaced UUID Workflow IDs with stable ID `temporal-trivia-active`, explicit
  `AllowDuplicate` reuse after completion, and `Fail` conflict behavior while
  running. Restarting the Rust controller restored the finished snapshot from
  a Temporal query, proving the TV state no longer depends only on process
  memory. The controller was left running at `127.0.0.1:3000`.
- The badge later appeared at `/dev/cu.usbmodem101`. Flashed the validated image
  to ESP32-S3 revision 0.2; the bootloader confirmed 16 MB flash and the 14 MB
  factory partition. Live boot passed the 8 MB PSRAM test, rendered through the
  OLED driver without error, connected to Wi-Fi at `192.168.1.103`, synchronized
  time, completed verified Temporal Cloud TLS, and polled
  `temporal-trivia-badges-v1` as `esp32-e83dc1f94c70` / `KEEN-SEAL-70`.
- Ran a real hardware round. The Workflow recorded `KEEN-SEAL-70 joined`, six
  completed questions, two correct answers, four wrong answers, one simulated
  crash, a recovery event, continued work after recovery, a final score of
  `-2`, and `KEEN-SEAL-70` as winner. This validates real Activity dispatch,
  button answers, scoring, retry/recovery, deadline completion, and final
  winner publication. The serial monitor was detached without stopping the
  badge Worker; the Mac controller remains live at `127.0.0.1:3000`.
