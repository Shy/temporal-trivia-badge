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
## 2026-08-17 — TV UI rebaseline

- Two visual passes based on generic PCB styling and the production backplate render were rejected because they treated the badge as decoration instead of matching its interface.
- Discarded both passes and rebaselined from `badge-ui-review-kathy-20260429-final.zip`, `GridMenuScreen.cpp`, `OLEDLayout.cpp`, and the firmware button-glyph generator.
- The new TV UI uses the badge's actual one-bit black/white language: thick rules, 2-column menu proportions, inverse-color selected cell, asymmetric rounded corners, compact status header/footer, and the exact 10x10 confirm-cluster bitmap geometry. It does not use the badge render as wallpaper.
- Inline JavaScript syntax and `cargo check --offline -p temporal-trivia-web --target aarch64-apple-darwin` passed. Browser inspection at the mirrored 1164x655 viewport showed no document overflow and preserved the durable finished-round state.

## 2026-08-17 — Badge deep sleep

- Added the original badge's idle-only hold-DOWN sleep gesture: the countdown
  arms after 250 ms and enters ESP32 deep sleep after 3 seconds. Releasing
  early returns to the Worker waiting screen.
- Sleep is gated by an Activity-active guard, so DOWN remains a trivia answer
  while a question owns the controls and cannot accidentally start shutdown.
- Wake uses the Echo hardware's diode-OR `INT_PWR_PIN` on RTC GPIO13, not the
  four button GPIOs directly. This lets any face button wake the badge while
  avoiding GPIO0/LEFT's boot-strapping role.
- The OLED blanks and receives display-off before sleep. The release gate
  prevents the held shutdown button from causing an immediate wake bounce.
- The release firmware built in 2m19s and fit the factory partition at
  8,015,360 / 14,680,064 bytes (54.60%). It was flashed to ESP32-S3 revision
  0.2 badge `e8:3d:c1:f9:4c:70`; live button sleep/wake confirmation is
  pending the physical press test.

## 2026-08-17 — Deep-sleep wake correction and recovery

- The first physical test entered deep sleep but did not wake. Comparing the
  implementation again with the original Echo `Power.cpp` found that the
  original arms both revision-dependent diode-OR wake lines, GPIO10 and
  GPIO13; the first implementation had copied only GPIO13.
- Corrected wake setup to configure both RTC GPIOs as pulled-up inputs and use
  an active-low EXT1 any-low mask. Individual face-button GPIOs remain
  intentionally excluded because LEFT is the GPIO0 boot strap.
- The corrected release image built at 8,015,504 / 14,680,064 bytes (54.60%).
  `espflash` then panicked while parsing serial data, and two esptool stub-mode
  attempts lost USB around 3%. A no-stub, uncompressed ROM-loader write
  completed all 8,015,504 bytes in 73.1 seconds and verified the flash hash.
- After that recovery write, the badge remained blank and silent while still
  enumerating as USB Serial/JTAG. An explicit ROM `run` reset also produced no
  application output. A clean USB power cycle is the next diagnostic gate;
  corrected sleep/wake behavior is not yet physically validated.

## 2026-08-17 — Direct-button wake verified

- The clean power cycle exposed `Invalid image block, can't boot.` The ROM
  loader had verified that the offline merged file was written accurately, but
  the merged file itself was invalid for this app layout. Reflashing the ELF
  through `espflash` with the explicit 16 MB partition table restored a normal
  boot; the no-stub recovery method should not reuse that merged artifact.
- A second physical test proved GPIO10/GPIO13 still did not respond to UP on
  this badge revision. Wake now arms the four actual active-low face-button
  GPIOs (0, 7, 17, 18) plus GPIO10/GPIO13 as revision fallbacks.
- The final build was 8,015,568 / 14,680,064 bytes (54.60%). On hardware, a
  3-second DOWN hold entered deep sleep, tapping UP caused an EXT1 wake and
  normal app boot, PSRAM passed, the OLED returned, Wi-Fi reacquired
  `192.168.1.103`, and the Worker resumed polling
  `temporal-trivia-badges-v1`.
- UP wake is physically verified. RIGHT and DOWN use the same direct-wake
  configuration. LEFT is also armed, but remains a separate validation case
  because its GPIO0 line is an ESP32 boot strap.

## 2026-08-17 — Durable failure, chaos, and round history

- Expanded the demo around four Temporal behaviors selected for the booth:
  heartbeat-timeout reassignment, a deliberately crashable Mac Worker,
  durable operator chaos controls, and history for completed rounds. Wrong
  answers now count as errors, apply the score penalty, and return the same
  question to Temporal instead of completing its Activity.
- Badge Activities keep the existing 5-second heartbeat timeout. Holding LEFT
  and RIGHT for 500 ms now abandons the local question and suppresses
  heartbeats for 6 seconds, allowing Temporal to time it out and dispatch the
  retry before the original badge resumes polling. Wrong answers use a
  retryable Activity failure and are also abandoned locally so that badge does
  not immediately reclaim the same question.
- Added durable Signals for 10 seconds of double points, 10 seconds of
  Rust-only questions, one 30-second extension, and sudden death. A live Cloud
  round accepted double points, Rust-only, and the extension; the deadline
  moved from 60 to 90 seconds and all three commands appeared in Workflow
  state. Sudden death is covered by the same implementation but was not
  exercised on hardware in this session.
- Added `run-web.sh` as a small supervisor. The operator crash endpoint exits
  with code 75, waits two seconds, and starts the Rust process again against
  the same Temporal state. Three live crash tests recovered successfully; the
  first cold rebuild took about 14 seconds and cached restarts took about four.
- Temporal Cloud rejected Search Attribute administration with `Request
  unauthorized.` The current API key can execute Workflows but cannot register
  namespace attributes. `configure-visibility.sh` preserves the optional
  registration path, while the default implementation stores a compact round
  summary in Workflow Memo and reads it through Visibility without elevated
  namespace permission.
- Two other history approaches were rejected. Cloud returned `Client specified
  an invalid argument` because this namespace does not support the attempted
  `ORDER BY` clause. Replaying old runs with the changed Workflow code exposed
  `[TMPRL1100] Nondeterminism error: Timer machine does not handle this event:
  HistoryEvent(id: 25, ActivityTaskScheduled)`. Memo avoids replaying old
  histories; pre-Memo runs are skipped.
- Rebuilt the mirrored-TV interface from scratch as a fixed 16:9, minimal PCB
  race board using local fonts and cropped real PCB layers. It retains stable
  first-seen lane positions, a single timer, restrained gold score flashes,
  frozen winner labels, a visible start test pad, and an operator drawer for
  chaos, history, and Mac Worker recovery. Browser checks passed at 1164x655,
  1920x1080, and a letterboxed 1400x700 viewport with no overflow.
- Host tests pass 7/7. The release firmware built at 8,040,176 / 14,680,064
  bytes (54.77%), flashed successfully, booted, joined Wi-Fi, and resumed
  polling Temporal Cloud as `KEEN-SEAL-70`.
- Remaining physical gate: no wrong-answer button press happened during the
  validation rounds, and only one badge was connected. The retry mechanics are
  implemented and host-tested, but a visible timeout handoff from one physical
  badge to another still needs a second badge.

## 2026-08-17 — Public setup and release preparation

- Reworked the README into a clean-room deployment path instead of relying on
  the private sibling `TrafficLight/.env`. Added ignored, repository-local
  Temporal and Wi-Fi examples, environment-variable overrides, ESP Rust
  toolchain installation, explicit firmware output and serial-port discovery,
  the required 16 MiB factory-partition flash command, controller startup,
  operator controls, badge controls, and verification commands.
- Made the build and flash scripts portable across normal `espup`, ESP-IDF, and
  Cargo tool locations. The scripts retain local tool fallbacks for this
  checkout, accept explicit overrides, and fail with an actionable install
  command when a tool is missing. The web supervisor now derives the host
  target instead of hard-coding Apple Silicon.
- Configuration fails before compiling firmware when a required Wi-Fi or
  Temporal value is blank. The current workspace keeps its legacy credential
  fallback, while a fresh clone uses `.env.temporal` or exported variables.
- Release checks passed: four shell scripts parse with `sh -n`, host tests pass
  7/7, formatting and `git diff --check` pass, and the release firmware rebuilt
  successfully in 2m18s with the existing seven dead-code warnings.
- GitHub publication is currently blocked outside the source tree: this local
  repository has no `origin`, and `gh auth status` reports that the active
  `Shy` token is invalid. No files were staged or pushed before resolving that
  ownership/authentication boundary.

## 2026-08-17 — GitHub authentication repaired

- The apparent recurring token expiration was an incomplete OAuth device flow,
  not an expiring stored token. `hosts.yml` retained the active `Shy` account
  name, but `gh auth token` could not retrieve a credential and the macOS
  Keychain had no GitHub CLI entry. The prior one-time device code then expired
  while the CLI waited for approval.
- Removed only the stale local `gh` account entry and completed a fresh browser
  login. Final verification reports `Shy (keyring)`, API user `Shy`, retrievable
  credentials, and the expected `repo`, `read:org`, and `gist` scopes.
- Switched Git operations to SSH. A direct GitHub SSH probe authenticated as
  `Shy`, so pushes no longer depend on HTTPS credential handling while GitHub
  API operations continue using the OAuth token stored in Keychain.

## 2026-08-17 — Public repository secret audit

- Before and after publishing, scanned the current tree and complete local Git
  history for Temporal API keys, Wi-Fi passwords, GitHub tokens, AWS-style
  keys, and private-key headers. No real credential material or private-key
  files were found; matches were limited to documented placeholder values and
  variable names.
- Confirmed `.env.temporal` and `firmware/.env.wifi` are ignored and have never
  appeared in the committed filename history. Hardened `.gitignore` with
  generic dotenv, private-key, certificate-bundle, and credentials-file rules
  while explicitly retaining sanitized `.example` templates.

## 2026-08-17 — Code-quality and durability review

- Ran OpenCodeReview 1.9.5 against 20 first-party files using GPT-5.4 through
  a process-only API key. The first attempt exposed an OCR provider-mode bug:
  `OCR_USE_ANTHROPIC=false` is treated as truthy; `0` selects OpenAI, while the
  native `--provider openai` path is the reliable configuration. Vendored SDK
  code and the large upstream trivia JSON were deliberately excluded.
- Removed the duplicated firmware/web game model and introduced the small
  `temporal-trivia-shared` crate. It owns the serialized contract, rejects
  out-of-range answer indices while deserializing, preserves the intentional
  pre-extension deadline default, and sorts tied winner callsigns explicitly.
- Replaced plaintext `cargo:rustc-env` credential directives with an ignored
  generated Rust config under `target/`. Firmware and controller now share a
  tested dotenv parser; explicit config-file overrides fail on missing paths.
  Credentials remain embedded in the flash image by design but no longer
  appear in verbose Cargo directives.
- Made NVS state updates transactional: the cached session changes only after
  flash persistence succeeds. Corrupt state now degrades to an empty session
  and is overwritten when the next game begins. A same-game deadline can move
  forward without clearing the badge's abandoned-question set.
- Badge result watchers now have owned task handles. A new game aborts the old
  watcher, and a completed watcher can be replaced, preventing stale rounds
  from overwriting the OLED. The local monotonic ceiling is 120 seconds, safely
  beyond Temporal's 95-second Activity timeout while still bounding a stale
  build-time clock fallback.
- Question generation now validates the full authored catalog before use,
  rejects undersized deck results, and removes panic-based shuffling. The web
  observer polls at 250 ms during healthy rounds and backs off to four seconds
  during repeated query failures; SSE lag is visible in logs. Round history
  scans 100 Memo-bearing executions, sorts locally by close time, and returns
  the newest 12 because this Cloud namespace previously rejected server-side
  `ORDER BY`.
- Review follow-up fixed all four medium findings from the first diff pass.
  A focused scan of the five files skipped by the token budget produced two
  high and six medium suggestions; local history sorting was accepted, while
  fixed-array, one-time startup I/O, intentional no-duplicate prompt filtering,
  serialized NVS access, and extension-deadline semantics were verified as
  context-dependent false positives or accepted embedded tradeoffs.
- Final host gates: strict Clippy passes with `-D warnings`; 11 tests pass
  across shared and web crates; formatting, shell syntax, and diff whitespace
  checks pass. The ESP32-S3 release firmware rebuilt twice in 2m13s and 2m12s;
  the final build completed without warnings. Physical behavior was not
  reflashed in this review because the firmware changes are structural and the
  target release build is the relevant gate; prior live badge validation still
  stands.
- After the focused follow-up renamed the extension deadline helper, the final
  ESP32-S3 release build passed again in 2m10s with no warnings. The supervised
  controller restarted as PID 40373, restored the frozen Cloud result, and its
  history endpoint returned the four Memo-bearing rounds newest-first.

## 2026-08-17 — Post-review badge flash validation

- Flashed the reviewed release ELF to the connected ESP32-S3 with
  `./flash-badge.sh`. Espflash identified the expected 16 MiB device and wrote
  the 8,041,616-byte application into the explicit 14,680,064-byte factory
  partition (54.78%).
- The physical badge completed a clean boot from the new image. The bootloader
  found 16 MiB flash, the 8 MiB PSRAM memory test passed, and the application
  retained its stable `KEEN-SEAL-70` callsign.
- Wi-Fi reconnected to the configured travel-router network. The Worker then
  logged that it was polling `temporal-trivia-badges-v1`, closing the
  post-review physical validation gate.
- Live play immediately exposed that the nominal 30/15/15/40 category mix was
  appended as four contiguous runs, so a normal round began with all 30 Rust
  questions. Each weighted batch is now shuffled before scheduling while
  retaining its exact category counts and unique-question guarantee. A
  regression test verifies that the opening questions mix at least three
  categories for the fixed test seed; the full host suite now passes 12 tests
  with strict Clippy warnings enabled.
- Clarified the hidden operator entry point in the README: click the `TP7` PCB
  test pad or press `O` to open the durable Workflow Signal controls.
- Exercised the supervised Mac crash endpoint during a live Cloud round. The
  controller process exited, `run-web.sh` restarted it with a new PID, and the
  state endpoint recovered the same active Workflow with its player, score,
  latest answer, double-points window, and used 30-second extension intact.

## 2026-08-17 — Visible Mac Worker recovery

- Replaced the single generic offline message with a four-stage recovery view:
  Worker stopped, supervisor restart, Temporal reconnect, and History restored.
  The scoreboard remains dimly visible behind it so the audience can see that
  the race state is frozen rather than reset.
- The browser begins the sequence as soon as the crash endpoint acknowledges
  the command. EventSource reconnection and a successful state refresh are the
  gate for the final recovered state; it remains visible for 2.5 seconds before
  returning to the race.
- Verified the sequence against the real supervised controller in the in-app
  browser. The restart stage rendered clearly at the mirrored 16:9 layout, the
  Rust process restarted, and the frozen winner returned unchanged after
  Temporal replay.

## 2026-08-17 — Component-owned setup guides

- Split the deployment documentation by component. `firmware/README.md` now
  owns ESP Rust installation, embedded configuration, release builds, serial
  discovery, flashing, controls, and physical verification. It explicitly
  targets the Temporal Replay 2026 Badge rather than a generic ESP32-S3 board.
- `web/README.md` now owns Temporal controller configuration, supervised server
  startup, scoreboard operation, Workflow Signal controls, optional Search
  Attributes, and host tests.
- Reduced the root README to shared architecture, requirements, Temporal Cloud
  configuration, repository navigation, credential hygiene, and common checks.
  It links directly into both component guides so a web-only user is no longer
  led through firmware flashing first.
