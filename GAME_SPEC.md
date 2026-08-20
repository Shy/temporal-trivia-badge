# Game contract

- The operator starts one 60-second game from the Mac web UI mirrored to a TV.
- Any badge polling the shared queue may join late; there is no frozen roster.
- Correct answers score `+1`. Wrong answers score `-1` and complete the
  Activity normally; the question is consumed. During double-points chaos
  those values become `+2` and `-2`. Activity retries are reserved for genuine
  Worker loss and heartbeat timeout.
- Holding LEFT+RIGHT for 500 ms simulates a Worker crash by suppressing
  heartbeats for six seconds. Temporal's five-second heartbeat timeout retries
  the Activity while the original badge is still unavailable. The badge
  refuses that question for the rest of the game, allowing another Worker to
  recover it. Panic itself scores `0`.
- The controller counts active ESP32 Activity pollers when a round starts. The
  Workflow keeps `max(1, active_badges - 1)` Activities outstanding, reserving
  one badge to pick up heartbeat-timeout retries. Retrying unfinished work is
  not a duplicate. The API may override the target for diagnostics.
- The global deadline cancels outstanding Activities for zero points. There is
  no per-badge timer. Ties create shared winners.
- Callsigns derive deterministically from the factory MAC and survive reboots.
  NVS retains the active game and abandoned-question IDs.
- From the idle Worker screen, holding DOWN for three seconds enters ESP32 deep
  sleep. Releasing before the countdown completes cancels it. Any face button
  wakes the badge through its direct RTC GPIO, with the hardware's shared
  revision wake lines retained as fallbacks. Sleep is not armed while an
  Activity owns the answer controls.
- The OLED uses wrapped questions, a compact 2x2 answer grid, and positional
  Nintendo-style glyphs rather than bare button letters.
- The TV is a fixed 16:9 race board: a header band carrying the round timer and
  live counters, the badge lanes, and a detail rail. Each lane contains
  callsign, rank, Worker state, score, and a score bar drawn as a routed trace
  whose length is relative to the current leader. Lanes never reorder during a
  round; the final board freezes in place and labels all tied winners. The rail
  carries the last resolved answer and a rolling feed of durable events, and
  switches to a round summary when the round closes. Above six badges the lanes
  split into two columns and the rail becomes a bottom band.
- Operator controls execute validated Workflow Updates for ten seconds of
  double points, ten seconds of Rust-only scheduling, sudden death on the next
  correct answer, and one `+30 seconds` timer extension. The three gameplay
  modifiers are mutually exclusive; the timer extension is independent. Each
  Update also records a monotonically numbered power-up notice in Workflow
  state. Awake badges query that durable state, vibrate, display it for 1.5
  seconds, suppress answer input during the overlay, and restore their prior
  question or waiting screen.
- The supervised Mac controller may be deliberately crashed. The browser keeps
  the frozen board visible while `run-web.sh` restarts the Rust process and the
  Workflow Worker rebuilds game state from Temporal history. Recovery steps
  advance only after observing process loss, a new process identity, a
  successful Temporal query, and a restored-state digest matching the frozen
  board.
- Finished round summaries are written to Temporal Memo and listed through
  Visibility; no game-history database is required. Typed Search Attributes
  are optional when the Cloud API key has namespace-operator permission.
- The deck is 30% Rust, 15% Temporal, 15% mental math, and 40% family-friendly
  general trivia. Temporal questions are mostly introductory. Questions may
  repeat in later games but never within the same game.
- Acceptance target: ten physical badges, with no protocol-level count limit.
- Demo-ready acceptance requires two physical badges proving that a fake crash
  produces a heartbeat timeout and moves the same question to the other badge
  as a higher real Temporal Activity attempt.
