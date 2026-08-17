pub use temporal_trivia_shared::{
    AnswerSpotlight, BADGE_TASK_QUEUE, BadgeAnswer, BadgeEvent, CHAOS_DURATION_MS, ChaosCommand,
    GAME_EXTENSION_MS, GAME_SECONDS, GameInput, GameSnapshot, GameStatus, PlayerScore, Question,
    QuestionTask, Reassignment, WEB_TASK_QUEUE,
};

use serde::{Deserialize, Serialize};

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
