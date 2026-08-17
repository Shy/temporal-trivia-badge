# Game contract

- The operator starts one 60-second game from the Mac web UI mirrored to a TV.
- Any badge polling the shared queue may join late; there is no frozen roster.
- Correct answers score `+1`; wrong answers score `-1`; panic scores `0`.
- Holding LEFT+RIGHT for 500 ms simulates a Worker crash by failing the Activity
  retryably. The badge recovers for 3 seconds and refuses that question for the
  rest of the game, allowing another badge to recover the work.
- The Workflow schedules each question at most once per game and maintains a
  backlog of `max(10, active_badges * 2)`. Retrying unfinished work is not a
  duplicate. The Mac UI may override the backlog target.
- The global deadline cancels outstanding Activities for zero points. There is
  no per-badge timer. Ties create shared winners.
- Callsigns derive deterministically from the factory MAC and survive reboots.
  NVS retains the active game and abandoned-question IDs.
- The OLED uses wrapped questions, a compact 2x2 answer grid, and positional
  Nintendo-style glyphs rather than bare button letters.
- The TV shows countdown, leaderboard, completed count, newest-answer
  spotlight, panic/recovery events, recent events, and the final tied podium.
- The deck is 30% Rust, 15% Temporal, 15% mental math, and 40% family-friendly
  general trivia. Temporal questions are mostly introductory. Questions may
  repeat in later games but never within the same game.
- Acceptance target: ten physical badges, with no protocol-level count limit.
