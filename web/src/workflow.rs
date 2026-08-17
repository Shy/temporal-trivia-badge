use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use temporalio_common::protos::coresdk::AsJsonPayloadExt;
use temporalio_common::protos::temporal::api::common::v1::RetryPolicy;
use temporalio_macros::{activity_definitions, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, SyncWorkflowContext, WorkflowContext, WorkflowContextView, WorkflowResult,
    activities::ActivityError,
};

use crate::model::{
    AnswerSpotlight, BADGE_TASK_QUEUE, BadgeAnswer, BadgeEvent, CHAOS_DURATION_MS, ChaosCommand,
    GAME_EXTENSION_MS, GameInput, GameSnapshot, GameStatus, PlayerScore, Question, QuestionTask,
    Reassignment, RoundMemo,
};

pub struct BadgeActivities;

#[activity_definitions]
impl BadgeActivities {
    #[activity(name = "trivia.answer_question")]
    fn answer_question(_task: QuestionTask) -> Result<BadgeAnswer, ActivityError> {
        unimplemented!()
    }
}

#[workflow]
#[derive(Default)]
pub struct GameWorkflow {
    snapshot: GameSnapshot,
    assignments: BTreeMap<String, BadgeEvent>,
    retry_reasons: BTreeMap<String, String>,
    questions: BTreeMap<String, Question>,
}

pub type GameWorkflowRun = <GameWorkflow as temporalio_common::HasWorkflowDefinition>::Run;

#[workflow_methods]
impl GameWorkflow {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: GameInput,
    ) -> WorkflowResult<GameSnapshot> {
        let started_unix_ms = unix_ms(ctx.workflow_time().unwrap_or(UNIX_EPOCH));
        let deadline_unix_ms = started_unix_ms + input.duration_seconds * 1_000;
        ctx.state_mut(|state| {
            state.assignments.clear();
            state.retry_reasons.clear();
            state.questions = input
                .questions
                .iter()
                .cloned()
                .map(|question| (question.id.clone(), question))
                .collect();
            state.snapshot = GameSnapshot {
                game_id: Some(input.game_id.clone()),
                status: GameStatus::Running,
                started_unix_ms: Some(started_unix_ms),
                deadline_unix_ms: Some(deadline_unix_ms),
                ..Default::default()
            };
            state.snapshot.push_event("Round started".to_owned());
        });
        if input.index_search_attributes {
            upsert_running_search_attributes(ctx, &input.game_id);
        }

        type PendingResult = (
            Question,
            Result<BadgeAnswer, temporalio_sdk::ActivityExecutionError>,
        );
        let mut pending: FuturesUnordered<futures::future::LocalBoxFuture<'static, PendingResult>> =
            FuturesUnordered::new();
        let mut available: VecDeque<Question> = input.questions.into();
        let activity_timeout = Duration::from_secs(input.duration_seconds + 35);

        loop {
            let now_unix_ms = workflow_unix_ms(ctx);
            let deadline_unix_ms = ctx
                .state(|state| state.snapshot.deadline_unix_ms)
                .unwrap_or(now_unix_ms);
            if now_unix_ms >= deadline_unix_ms {
                break;
            }
            let rust_only = ctx.state(|state| {
                state
                    .snapshot
                    .chaos
                    .rust_only_until_unix_ms
                    .is_some_and(|until| until > now_unix_ms)
            });
            let target = ctx.state(|state| state.snapshot.target_backlog(input.backlog_override));
            while pending.len() < target {
                let Some(question) = take_next_question(&mut available, rust_only) else {
                    break;
                };
                let task = QuestionTask {
                    game_id: input.game_id.clone(),
                    deadline_unix_ms,
                    // The extension card is single-use, so this upper bound
                    // keeps an in-flight badge alive across a possible +30s.
                    max_deadline_unix_ms: started_unix_ms
                        + input.duration_seconds * 1_000
                        + GAME_EXTENSION_MS,
                    question: question.clone(),
                };
                let activity_ctx = (*ctx).clone();
                pending.push(
                    async move {
                        let result = activity_ctx
                            .start_activity(
                                BadgeActivities::answer_question,
                                task,
                                ActivityOptions::with_schedule_to_close_timeout(activity_timeout)
                                    .heartbeat_timeout(Duration::from_secs(5))
                                    .retry_policy(RetryPolicy {
                                        initial_interval: Some(prost_wkt_types::Duration {
                                            seconds: 0,
                                            nanos: 250_000_000,
                                        }),
                                        backoff_coefficient: 1.0,
                                        maximum_interval: Some(prost_wkt_types::Duration {
                                            seconds: 1,
                                            nanos: 0,
                                        }),
                                        ..Default::default()
                                    })
                                    .task_queue(BADGE_TASK_QUEUE)
                                    .activity_id(question.id.clone())
                                    .build(),
                            )
                            .await;
                        (question, result)
                    }
                    .boxed_local(),
                );
                ctx.state_mut(|state| state.snapshot.scheduled_questions += 1);
            }

            if pending.is_empty() && available.is_empty() {
                ctx.state_mut(|state| {
                    state
                        .snapshot
                        .push_event("Question deck exhausted".to_owned())
                });
                break;
            }

            let tick_duration = Duration::from_millis((deadline_unix_ms - now_unix_ms).min(1_000));
            let tick_ctx = (*ctx).clone();
            let mut tick = async move { tick_ctx.timer(tick_duration).await }
                .boxed_local()
                .fuse();

            if pending.is_empty() {
                tick.await;
                continue;
            }

            futures::select_biased! {
                _ = tick => continue,
                completed = pending.next().fuse() => {
                    let Some((question, result)) = completed else { break };
                    match result {
                        Ok(answer) => {
                            let now_unix_ms = workflow_unix_ms(ctx);
                            if record_answer(ctx, question, answer, now_unix_ms) {
                                break;
                            }
                        }
                        Err(error) => ctx.state_mut(|state| {
                            state.snapshot.push_event(format!(
                                "Question {} closed without an answer: {error}", question.id
                            ));
                        }),
                    }
                }
            }
        }

        drop(pending);
        ctx.state_mut(|state| state.snapshot.finish());
        let round_memo = ctx.state(|state| RoundMemo::from(&state.snapshot));
        ctx.upsert_memo([(
            "TriviaRoundSummary".to_owned(),
            round_memo
                .as_json_payload()
                .expect("round summary memo payload"),
        )]);
        if input.index_search_attributes {
            upsert_finished_search_attributes(ctx);
        }
        Ok(ctx.state(|state| state.snapshot.clone()))
    }

    #[signal]
    pub fn badge_started(&mut self, _ctx: &mut SyncWorkflowContext<Self>, event: BadgeEvent) {
        if let Some(previous) = self.assignments.get(&event.question_id)
            && previous.badge_id != event.badge_id
            && let Some(reason) = self.retry_reasons.get(&event.question_id).cloned()
        {
            let reassignment = Reassignment {
                question_id: event.question_id.clone(),
                from_callsign: previous.callsign.clone(),
                to_callsign: event.callsign.clone(),
                reason,
            };
            self.snapshot.reassignments += 1;
            self.snapshot.latest_reassignment = Some(reassignment.clone());
            self.snapshot.push_event(format!(
                "Temporal retried {}: {} -> {} ({})",
                reassignment.question_id,
                reassignment.from_callsign,
                reassignment.to_callsign,
                reassignment.reason
            ));
            self.retry_reasons.remove(&event.question_id);
        }
        self.assignments
            .insert(event.question_id.clone(), event.clone());
        if !self.snapshot.players.contains_key(&event.badge_id) {
            self.snapshot.players.insert(
                event.badge_id.clone(),
                PlayerScore {
                    badge_id: event.badge_id,
                    callsign: event.callsign.clone(),
                    ..Default::default()
                },
            );
            self.snapshot
                .push_event(format!("{} joined", event.callsign));
        }
    }

    #[signal]
    pub fn panic_event(&mut self, _ctx: &mut SyncWorkflowContext<Self>, event: BadgeEvent) {
        self.retry_reasons
            .insert(event.question_id.clone(), "heartbeat timeout".to_owned());
        let player = self
            .snapshot
            .players
            .entry(event.badge_id.clone())
            .or_insert_with(|| PlayerScore {
                badge_id: event.badge_id,
                callsign: event.callsign.clone(),
                ..Default::default()
            });
        player.panics += 1;
        self.snapshot.push_event(format!(
            "{} crashed on {}",
            event.callsign, event.question_id
        ));
    }

    #[signal]
    pub fn recovered(&mut self, _ctx: &mut SyncWorkflowContext<Self>, event: BadgeEvent) {
        self.snapshot
            .push_event(format!("{} recovered; question returned", event.callsign));
    }

    #[signal]
    pub fn wrong_answer(&mut self, ctx: &mut SyncWorkflowContext<Self>, answer: BadgeAnswer) {
        let Some(question) = self.questions.get(&answer.question_id) else {
            self.snapshot.push_event(format!(
                "Rejected unknown question result from {}",
                answer.callsign
            ));
            return;
        };
        if answer.selected_index > 3 || answer.selected_index == question.correct_index {
            self.snapshot.push_event(format!(
                "Rejected malformed wrong-answer signal from {}",
                answer.callsign
            ));
            return;
        }
        let now_unix_ms = unix_ms(ctx.workflow_time().unwrap_or(UNIX_EPOCH));
        let points = active_points(&self.snapshot, now_unix_ms);
        let player = self
            .snapshot
            .players
            .entry(answer.badge_id.clone())
            .or_insert_with(|| PlayerScore {
                badge_id: answer.badge_id.clone(),
                callsign: answer.callsign.clone(),
                ..Default::default()
            });
        player.score -= points;
        player.wrong += 1;
        let score = player.score;
        self.snapshot.latest_answer = Some(AnswerSpotlight {
            question: question.prompt.clone(),
            correct_answer: question.answers[question.correct_index as usize].clone(),
            callsign: answer.callsign.clone(),
            was_correct: false,
            score,
            points,
        });
        self.snapshot.push_event(format!(
            "{} missed {} ({}) — Temporal retrying",
            answer.callsign, answer.question_id, -points
        ));
        self.retry_reasons
            .insert(answer.question_id, "wrong answer".to_owned());
    }

    #[signal]
    pub fn apply_chaos(&mut self, ctx: &mut SyncWorkflowContext<Self>, command: ChaosCommand) {
        if self.snapshot.status != GameStatus::Running {
            return;
        }
        let now_unix_ms = unix_ms(ctx.workflow_time().unwrap_or(UNIX_EPOCH));
        match command {
            ChaosCommand::DoublePoints => {
                self.snapshot.chaos.double_points_until_unix_ms =
                    Some(now_unix_ms + CHAOS_DURATION_MS);
                self.snapshot
                    .push_event("CHAOS: double points for 10 seconds".to_owned());
            }
            ChaosCommand::RustOnly => {
                self.snapshot.chaos.rust_only_until_unix_ms = Some(now_unix_ms + CHAOS_DURATION_MS);
                self.snapshot
                    .push_event("CHAOS: Rust questions only for 10 seconds".to_owned());
            }
            ChaosCommand::SuddenDeath => {
                self.snapshot.chaos.sudden_death = true;
                self.snapshot
                    .push_event("CHAOS: next correct answer ends the round".to_owned());
            }
            ChaosCommand::ExtendThirtySeconds if !self.snapshot.chaos.extension_used => {
                self.snapshot.chaos.extension_used = true;
                self.snapshot.deadline_unix_ms = self
                    .snapshot
                    .deadline_unix_ms
                    .map(|deadline| deadline + GAME_EXTENSION_MS);
                self.snapshot
                    .push_event("CHAOS: Temporal timer extended by 30 seconds".to_owned());
            }
            ChaosCommand::ExtendThirtySeconds => {
                self.snapshot
                    .push_event("CHAOS: +30 seconds was already used".to_owned());
            }
        }
    }

    #[query]
    pub fn snapshot(&self, _ctx: &WorkflowContextView) -> GameSnapshot {
        self.snapshot.clone()
    }
}

fn record_answer(
    ctx: &mut WorkflowContext<GameWorkflow>,
    question: Question,
    answer: BadgeAnswer,
    now_unix_ms: u64,
) -> bool {
    ctx.state_mut(|state| {
        if answer.question_id != question.id || answer.selected_index > 3 {
            state.snapshot.push_event(format!(
                "Rejected malformed answer from {}",
                answer.callsign
            ));
            return false;
        }
        let was_correct = answer.selected_index == question.correct_index;
        let points = active_points(&state.snapshot, now_unix_ms);
        let score = {
            let player = state
                .snapshot
                .players
                .entry(answer.badge_id.clone())
                .or_insert_with(|| PlayerScore {
                    badge_id: answer.badge_id,
                    callsign: answer.callsign.clone(),
                    ..Default::default()
                });
            if was_correct {
                player.score += points;
                player.correct += 1;
            } else {
                player.score -= points;
                player.wrong += 1;
            }
            player.score
        };
        state.snapshot.completed_questions += 1;
        state.snapshot.latest_answer = Some(AnswerSpotlight {
            question: question.prompt,
            correct_answer: question.answers[question.correct_index as usize].clone(),
            callsign: answer.callsign.clone(),
            was_correct,
            score,
            points,
        });
        state.snapshot.push_event(format!(
            "{} answered {} ({:+})",
            answer.callsign,
            if was_correct { "correctly" } else { "wrong" },
            if was_correct { points } else { -points }
        ));
        state.snapshot.chaos.sudden_death && was_correct
    })
}

fn active_points(snapshot: &GameSnapshot, now_unix_ms: u64) -> i32 {
    if snapshot
        .chaos
        .double_points_until_unix_ms
        .is_some_and(|until| until > now_unix_ms)
    {
        2
    } else {
        1
    }
}

fn take_next_question(available: &mut VecDeque<Question>, rust_only: bool) -> Option<Question> {
    if rust_only {
        let index = available
            .iter()
            .position(|question| question.category == "rust")?;
        available.remove(index)
    } else {
        available.pop_front()
    }
}

fn workflow_unix_ms(ctx: &WorkflowContext<GameWorkflow>) -> u64 {
    unix_ms(ctx.workflow_time().unwrap_or(UNIX_EPOCH))
}

fn upsert_running_search_attributes(ctx: &WorkflowContext<GameWorkflow>, game_id: &str) {
    ctx.upsert_search_attributes([
        (
            "TriviaGameId".to_owned(),
            game_id
                .to_owned()
                .as_json_payload()
                .expect("game id payload"),
        ),
        (
            "TriviaStatus".to_owned(),
            "Running"
                .to_owned()
                .as_json_payload()
                .expect("status payload"),
        ),
    ]);
}

fn upsert_finished_search_attributes(ctx: &WorkflowContext<GameWorkflow>) {
    let snapshot = ctx.state(|state| state.snapshot.clone());
    let correct = snapshot
        .players
        .values()
        .map(|player| player.correct)
        .sum::<u32>();
    let wrong = snapshot
        .players
        .values()
        .map(|player| player.wrong)
        .sum::<u32>();
    let panics = snapshot
        .players
        .values()
        .map(|player| player.panics)
        .sum::<u32>();
    ctx.upsert_search_attributes([
        (
            "TriviaStatus".to_owned(),
            "Finished"
                .to_owned()
                .as_json_payload()
                .expect("status payload"),
        ),
        (
            "TriviaWinners".to_owned(),
            snapshot.winners.as_json_payload().expect("winners payload"),
        ),
        (
            "TriviaBadgeCount".to_owned(),
            (snapshot.players.len() as i64)
                .as_json_payload()
                .expect("badge count payload"),
        ),
        (
            "TriviaCorrectAnswers".to_owned(),
            (correct as i64).as_json_payload().expect("correct payload"),
        ),
        (
            "TriviaWrongAnswers".to_owned(),
            (wrong as i64).as_json_payload().expect("wrong payload"),
        ),
        (
            "TriviaCrashes".to_owned(),
            (panics as i64).as_json_payload().expect("crash payload"),
        ),
        (
            "TriviaReassignments".to_owned(),
            (snapshot.reassignments as i64)
                .as_json_payload()
                .expect("reassignment payload"),
        ),
    ]);
}

fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str, category: &str) -> Question {
        Question {
            id: id.to_owned(),
            category: category.to_owned(),
            difficulty: "easy".to_owned(),
            prompt: format!("Question {id}"),
            answers: [
                "A".to_owned(),
                "B".to_owned(),
                "C".to_owned(),
                "D".to_owned(),
            ],
            correct_index: 0,
        }
    }

    #[test]
    fn unix_time_conversion_is_milliseconds() {
        assert_eq!(unix_ms(UNIX_EPOCH + Duration::from_secs(3)), 3_000);
    }

    #[test]
    fn rust_only_scheduling_preserves_other_questions_for_later() {
        let mut available = VecDeque::from([
            question("general-1", "general"),
            question("rust-1", "rust"),
            question("math-1", "math"),
        ]);
        assert_eq!(
            take_next_question(&mut available, true).map(|question| question.id),
            Some("rust-1".to_owned())
        );
        assert_eq!(
            take_next_question(&mut available, false).map(|question| question.id),
            Some("general-1".to_owned())
        );
        assert_eq!(
            available.front().map(|question| question.id.as_str()),
            Some("math-1")
        );
    }

    #[test]
    fn double_points_expires_on_workflow_time() {
        let mut snapshot = GameSnapshot::default();
        snapshot.chaos.double_points_until_unix_ms = Some(20_000);
        assert_eq!(active_points(&snapshot, 19_999), 2);
        assert_eq!(active_points(&snapshot, 20_000), 1);
    }
}
