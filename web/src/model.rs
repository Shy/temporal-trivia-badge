use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const BADGE_TASK_QUEUE: &str = "temporal-trivia-badges-v1";
pub const WEB_TASK_QUEUE: &str = "temporal-trivia-web-v1";
pub const GAME_SECONDS: u64 = 60;

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameInput {
    pub game_id: String,
    pub questions: Vec<Question>,
    pub duration_seconds: u64,
    pub backlog_override: Option<usize>,
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
