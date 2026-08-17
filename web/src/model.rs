use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const BADGE_TASK_QUEUE: &str = "temporal-trivia-badges-v1";
pub const WEB_TASK_QUEUE: &str = "temporal-trivia-web-v1";
pub const GAME_SECONDS: u64 = 60;
pub const CHAOS_DURATION_MS: u64 = 10_000;
pub const GAME_EXTENSION_MS: u64 = 30_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub category: String,
    pub difficulty: String,
    pub prompt: String,
    pub answers: [String; 4],
    pub correct_index: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuestionTask {
    pub game_id: String,
    pub deadline_unix_ms: u64,
    #[serde(default)]
    pub max_deadline_unix_ms: u64,
    pub question: Question,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BadgeAnswer {
    pub badge_id: String,
    pub callsign: String,
    pub question_id: String,
    pub selected_index: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BadgeEvent {
    pub badge_id: String,
    pub callsign: String,
    pub question_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChaosCommand {
    DoublePoints,
    RustOnly,
    SuddenDeath,
    ExtendThirtySeconds,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChaosState {
    pub double_points_until_unix_ms: Option<u64>,
    pub rust_only_until_unix_ms: Option<u64>,
    pub sudden_death: bool,
    pub extension_used: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reassignment {
    pub question_id: String,
    pub from_callsign: String,
    pub to_callsign: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameInput {
    pub game_id: String,
    pub questions: Vec<Question>,
    pub duration_seconds: u64,
    pub backlog_override: Option<usize>,
    #[serde(default)]
    pub index_search_attributes: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerScore {
    pub badge_id: String,
    pub callsign: String,
    pub score: i32,
    pub correct: u32,
    pub wrong: u32,
    pub panics: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameStatus {
    Waiting,
    Running,
    Finished,
}

impl Default for GameStatus {
    fn default() -> Self {
        Self::Waiting
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnswerSpotlight {
    pub question: String,
    pub correct_answer: String,
    pub callsign: String,
    pub was_correct: bool,
    pub score: i32,
    #[serde(default = "default_points")]
    pub points: i32,
}

const fn default_points() -> i32 {
    1
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameSnapshot {
    pub game_id: Option<String>,
    pub status: GameStatus,
    pub started_unix_ms: Option<u64>,
    pub deadline_unix_ms: Option<u64>,
    pub completed_questions: u32,
    pub scheduled_questions: u32,
    pub players: BTreeMap<String, PlayerScore>,
    pub latest_answer: Option<AnswerSpotlight>,
    pub events: Vec<String>,
    pub winners: Vec<String>,
    #[serde(default)]
    pub reassignments: u32,
    #[serde(default)]
    pub latest_reassignment: Option<Reassignment>,
    #[serde(default)]
    pub chaos: ChaosState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoundMemo {
    pub game_id: String,
    pub winners: Vec<String>,
    pub badge_count: i64,
    pub correct_answers: i64,
    pub wrong_answers: i64,
    pub crashes: i64,
    pub reassignments: i64,
}

impl From<&GameSnapshot> for RoundMemo {
    fn from(snapshot: &GameSnapshot) -> Self {
        Self {
            game_id: snapshot.game_id.clone().unwrap_or_default(),
            winners: snapshot.winners.clone(),
            badge_count: snapshot.players.len() as i64,
            correct_answers: snapshot
                .players
                .values()
                .map(|player| i64::from(player.correct))
                .sum(),
            wrong_answers: snapshot
                .players
                .values()
                .map(|player| i64::from(player.wrong))
                .sum(),
            crashes: snapshot
                .players
                .values()
                .map(|player| i64::from(player.panics))
                .sum(),
            reassignments: i64::from(snapshot.reassignments),
        }
    }
}

impl GameSnapshot {
    pub fn push_event(&mut self, event: String) {
        self.events.push(event);
        if self.events.len() > 12 {
            self.events.remove(0);
        }
    }

    pub fn target_backlog(&self, override_value: Option<usize>) -> usize {
        override_value.unwrap_or_else(|| 10.max(self.players.len() * 2))
    }

    pub fn finish(&mut self) {
        self.status = GameStatus::Finished;
        let high_score = self.players.values().map(|player| player.score).max();
        self.winners = high_score
            .map(|score| {
                self.players
                    .values()
                    .filter(|player| player.score == score)
                    .map(|player| player.callsign.clone())
                    .collect()
            })
            .unwrap_or_default();
        if self.winners.is_empty() {
            self.push_event("Round finished with no answers".to_owned());
        } else {
            self.push_event(format!("Winner: {}", self.winners.join(" + ")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backlog_scales_from_ten() {
        let mut state = GameSnapshot::default();
        assert_eq!(state.target_backlog(None), 10);
        for index in 0..8 {
            state.players.insert(
                index.to_string(),
                PlayerScore {
                    badge_id: index.to_string(),
                    callsign: format!("BADGE-{index}"),
                    ..Default::default()
                },
            );
        }
        assert_eq!(state.target_backlog(None), 16);
        assert_eq!(state.target_backlog(Some(33)), 33);
    }

    #[test]
    fn finish_allows_ties() {
        let mut state = GameSnapshot::default();
        for name in ["FERRIS-01", "CRAB-02"] {
            state.players.insert(
                name.to_owned(),
                PlayerScore {
                    badge_id: name.to_owned(),
                    callsign: name.to_owned(),
                    score: 4,
                    ..Default::default()
                },
            );
        }
        state.finish();
        assert_eq!(state.winners, ["CRAB-02", "FERRIS-01"]);
    }
}
