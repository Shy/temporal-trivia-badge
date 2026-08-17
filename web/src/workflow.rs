use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use temporalio_macros::{activity_definitions, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, SyncWorkflowContext, WorkflowContext, WorkflowContextView, WorkflowResult,
    activities::ActivityError,
};

use crate::model::{
    AnswerSpotlight, BADGE_TASK_QUEUE, BadgeAnswer, BadgeEvent, GameInput, GameSnapshot,
    GameStatus, PlayerScore, Question, QuestionTask,
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
}

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
            state.snapshot = GameSnapshot {
                game_id: Some(input.game_id.clone()),
                status: GameStatus::Running,
                started_unix_ms: Some(started_unix_ms),
                deadline_unix_ms: Some(deadline_unix_ms),
                ..Default::default()
            };
            state.snapshot.push_event("Round started".to_owned());
        });

        type PendingResult = (
            Question,
            Result<BadgeAnswer, temporalio_sdk::ActivityExecutionError>,
        );
        let mut pending: FuturesUnordered<futures::future::LocalBoxFuture<'static, PendingResult>> =
            FuturesUnordered::new();
        let mut next_question = 0_usize;
        let game_duration = Duration::from_secs(input.duration_seconds);
        let activity_timeout = Duration::from_secs(input.duration_seconds + 5);
        let timer_ctx = (*ctx).clone();
        let mut timer = async move { timer_ctx.timer(game_duration).await }
            .boxed_local()
            .fuse();

        loop {
            let target = ctx.state(|state| state.snapshot.target_backlog(input.backlog_override));
            while pending.len() < target && next_question < input.questions.len() {
                let question = input.questions[next_question].clone();
                next_question += 1;
                let task = QuestionTask {
                    game_id: input.game_id.clone(),
                    deadline_unix_ms,
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

            if pending.is_empty() {
                ctx.state_mut(|state| {
                    state
                        .snapshot
                        .push_event("Question deck exhausted".to_owned())
                });
                break;
            }

            futures::select_biased! {
                _ = timer => break,
                completed = pending.next().fuse() => {
                    let Some((question, result)) = completed else { break };
                    match result {
                        Ok(answer) => record_answer(ctx, question, answer),
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
        Ok(ctx.state(|state| state.snapshot.clone()))
    }

    #[signal]
    pub fn badge_started(&mut self, _ctx: &mut SyncWorkflowContext<Self>, event: BadgeEvent) {
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

    #[query]
    pub fn snapshot(&self, _ctx: &WorkflowContextView) -> GameSnapshot {
        self.snapshot.clone()
    }
}

fn record_answer(ctx: &mut WorkflowContext<GameWorkflow>, question: Question, answer: BadgeAnswer) {
    ctx.state_mut(|state| {
        if answer.question_id != question.id || answer.selected_index > 3 {
            state.snapshot.push_event(format!(
                "Rejected malformed answer from {}",
                answer.callsign
            ));
            return;
        }
        let was_correct = answer.selected_index == question.correct_index;
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
                player.score += 1;
                player.correct += 1;
            } else {
                player.score -= 1;
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
        });
        state.snapshot.push_event(format!(
            "{} answered {} ({:+})",
            answer.callsign,
            if was_correct { "correctly" } else { "wrong" },
            if was_correct { 1 } else { -1 }
        ));
    });
}

fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_time_conversion_is_milliseconds() {
        assert_eq!(unix_ms(UNIX_EPOCH + Duration::from_secs(3)), 3_000);
    }
}
