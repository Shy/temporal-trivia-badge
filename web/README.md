# Web controller and scoreboard

This directory contains the Rust Temporal Workflow Worker, Axum operator
server, trivia deck, and fixed 16:9 scoreboard. The controller starts rounds,
schedules question Activities for Replay 2026 badges, observes durable game
state, and serves the operator UI at `127.0.0.1:3000`.

Run the commands below from the repository root.

## Requirements

- Rust installed with `rustup`.
- A Temporal Cloud namespace and API key.
- A macOS or Linux laptop that can reach Temporal Cloud.
- A browser suitable for mirroring to the event display.

The web controller can run without a connected badge, but a physical round
needs at least one flashed Replay 2026 badge polling the shared Task Queue. See
the [firmware guide](../firmware/README.md) to prepare one.

## Configure Temporal Cloud

Complete the root [shared Temporal configuration](../README.md#shared-temporal-configuration).
The controller reads that configuration when `run-web.sh` starts it.

## Run the controller

Start the included restart supervisor:

```sh
./run-web.sh
```

Open <http://127.0.0.1:3000> and mirror that browser window to the TV. Use
`Ctrl+C` in the terminal to stop the controller.

Always use `run-web.sh` for the demo. It marks the Worker as supervised and
restarts it after the operator deliberately crashes the Mac process. The
scoreboard shows Worker stopped, supervisor restart, Temporal reconnect, and
History restored while Temporal replays the active Workflow.

## Run a game

Click **START ROUND** after badges are polling. Only one round can run at a
time, and badges that begin polling during an active round join automatically.

Open the operator drawer with the small **TP7** test pad in the bottom-right
corner or the `O` keyboard shortcut. Its Workflow Signals provide:

- Double points for 10 seconds.
- Rust-only scheduling for 10 seconds.
- Sudden death, where the next correct answer ends the round.
- One 30-second extension.
- A supervised Mac Worker crash and automatic restart demonstration.

Completed rounds are stored in Workflow Memo and listed through Temporal
Visibility. No database or namespace changes are required.

## Optional Search Attributes

Typed Search Attributes are optional and require an API key with
namespace-operator permission:

```sh
./configure-visibility.sh
TRIVIA_SEARCH_ATTRIBUTES=1 ./run-web.sh
```

The game and round history work without them.

## Test the web and shared crates

```sh
host_target=$(rustc -vV | awk '/^host:/ { print $2 }')
cargo test --offline -p temporal-trivia-shared -p temporal-trivia-web --target "$host_target"
cargo clippy --offline -p temporal-trivia-shared -p temporal-trivia-web \
  --target "$host_target" --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

The question-pool tests verify the category mix, badge-size constraints, unique
questions, and answer indexes. Workflow tests cover durable timing, chaos
Signals, retry scheduling, and shared payload validation.

See the root [game specification](../GAME_SPEC.md) for scoring and retry rules
and the [engineering journal](../blog.md) for live recovery validation.
