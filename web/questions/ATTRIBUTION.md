# Question attribution

`source/open-trivia-db.json` is a snapshot from
[leakyhose/open-trivia-script-data](https://github.com/leakyhose/open-trivia-script-data),
which retrieved verified questions from the
[Open Trivia Database](https://opentdb.com/).

Open Trivia Database states that its question data is licensed under the
[Creative Commons Attribution-ShareAlike 4.0 International license](https://creativecommons.org/licenses/by-sa/4.0/).
The game decodes, filters, deduplicates, and shuffles that data. The unmodified
source snapshot remains here to preserve provenance.

Rust and Temporal questions in `web/src/questions.rs` are original to this
project. Temporal facts should be checked against current official
documentation before an event build. Mental-math questions are generated
deterministically by the game.
