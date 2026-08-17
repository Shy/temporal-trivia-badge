mod model;
mod questions;
mod workflow;

use std::{
    collections::HashMap,
    convert::Infallible,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, TlsOptions, WorkflowExecution,
    WorkflowHandle, WorkflowListOptions, WorkflowQueryOptions, WorkflowSignalOptions,
    WorkflowStartOptions,
};
use temporalio_common::protos::temporal::api::enums::v1::{
    WorkflowIdConflictPolicy, WorkflowIdReusePolicy,
};
use temporalio_common::{telemetry::TelemetryOptions, worker::WorkerTaskTypes};
use temporalio_sdk::{Worker, WorkerOptions};
use temporalio_sdk_core::{CoreRuntime, RuntimeOptions, Url};
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::{
    model::{
        ChaosCommand, GAME_SECONDS, GameInput, GameSnapshot, GameStatus, RoundMemo, WEB_TASK_QUEUE,
    },
    workflow::{GameWorkflow, GameWorkflowRun},
};

const INDEX_HTML: &str = include_str!("../static/index.html");
const ACTIVE_WORKFLOW_ID: &str = "temporal-trivia-active";
const SUPERVISOR_RESTART_EXIT: i32 = 75;
const WORKFLOW_POLL_INTERVAL: Duration = Duration::from_millis(250);
const WORKFLOW_MAX_ERROR_BACKOFF: Duration = Duration::from_secs(4);
const MAX_BACKLOG_OVERRIDE: usize = 100;
const ROUND_HISTORY_LIMIT: usize = 12;
const ROUND_HISTORY_SCAN_LIMIT: usize = 100;

#[derive(Clone)]
struct AppState {
    client: Client,
    snapshot: Arc<RwLock<GameSnapshot>>,
    active_workflow: Arc<Mutex<Option<String>>>,
    events: broadcast::Sender<String>,
}

#[derive(Default, Deserialize)]
struct StartRequest {
    backlog_override: Option<usize>,
}

#[derive(Debug, Serialize)]
struct RoundSummary {
    game_id: String,
    run_id: String,
    closed_unix_ms: Option<u64>,
    winners: Vec<String>,
    badge_count: i64,
    correct_answers: i64,
    wrong_answers: i64,
    crashes: i64,
    reassignments: i64,
}

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, self.1).into_response()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = connect_cloud().await?;
    let runtime = CoreRuntime::new_assume_tokio(
        RuntimeOptions::builder()
            .telemetry_options(TelemetryOptions::builder().build())
            .build()
            .map_err(|error| anyhow!(error))?,
    )?;
    let worker_options = WorkerOptions::new(WEB_TASK_QUEUE)
        .register_workflow::<GameWorkflow>()?
        .task_types(WorkerTaskTypes::workflow_only())
        // The SDK's detector treats FuturesUnordered's forwarding wakers as
        // external even when every contained future is an SDK Activity/timer.
        // Core's own FuturesUnordered test uses this same opt-out.
        .detect_nondeterministic_futures(false)
        .build();
    let mut worker = Worker::new(&runtime, client.clone(), worker_options)
        .map_err(|error| anyhow!(error.to_string()))?;

    let (events, _) = broadcast::channel(128);
    let state = AppState {
        client,
        snapshot: Arc::new(RwLock::new(GameSnapshot::default())),
        active_workflow: Arc::new(Mutex::new(None)),
        events,
    };
    resume_active_game(state.clone());
    let app = Router::new()
        .route("/", get(index))
        .route("/api/state", get(current_state))
        .route("/api/events", get(event_stream))
        .route("/api/start", post(start_game))
        .route("/api/chaos/{command}", post(apply_chaos))
        .route("/api/history", get(round_history))
        .route("/api/crash-worker", post(crash_worker))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("Temporal Trivia controller: http://127.0.0.1:3000");

    let server = async move {
        axum::serve(listener, app)
            .await
            .map_err(|error| anyhow!(error))
    };
    tokio::try_join!(worker.run(), server)?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn current_state(State(state): State<AppState>) -> Json<GameSnapshot> {
    Json(state.snapshot.read().await.clone())
}

async fn event_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = BroadcastStream::new(state.events.subscribe());
    let stream = receiver.filter_map(|message| async move {
        match message {
            Ok(json) => Some(Ok(Event::default().data(json))),
            Err(error) => {
                eprintln!("SSE subscriber lagged and skipped updates: {error}");
                None
            }
        }
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn start_game(
    State(state): State<AppState>,
    Json(request): Json<StartRequest>,
) -> Result<Json<GameSnapshot>, ApiError> {
    if request
        .backlog_override
        .is_some_and(|value| !(1..=MAX_BACKLOG_OVERRIDE).contains(&value))
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("backlog override must be between 1 and {MAX_BACKLOG_OVERRIDE}"),
        ));
    }
    let deck = questions::build_deck(rand::random(), 500)
        .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let game_id = format!("trivia-{}", Uuid::new_v4().simple());
    {
        let mut active = state.active_workflow.lock().await;
        if active.is_some() {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "a game is already running".to_owned(),
            ));
        }
        // Reserve the single-game slot before the asynchronous Cloud call so
        // two simultaneous button presses cannot start two Workflows.
        *active = Some(game_id.clone());
    }

    let input = GameInput {
        game_id: game_id.clone(),
        questions: deck,
        duration_seconds: GAME_SECONDS,
        backlog_override: request.backlog_override,
        index_search_attributes: std::env::var("TRIVIA_SEARCH_ATTRIBUTES").as_deref() == Ok("1"),
    };
    let handle_result = state
        .client
        .start_workflow(
            GameWorkflow::run,
            input,
            WorkflowStartOptions::new(WEB_TASK_QUEUE, ACTIVE_WORKFLOW_ID)
                .id_reuse_policy(WorkflowIdReusePolicy::AllowDuplicate)
                .id_conflict_policy(WorkflowIdConflictPolicy::Fail)
                .build(),
        )
        .await;
    let handle = match handle_result {
        Ok(handle) => handle,
        Err(error) => {
            let mut active = state.active_workflow.lock().await;
            if active.as_deref() == Some(&game_id) {
                *active = None;
            }
            return Err(ApiError(StatusCode::BAD_GATEWAY, error.to_string()));
        }
    };

    let starting = GameSnapshot {
        game_id: Some(game_id.clone()),
        status: GameStatus::Running,
        ..Default::default()
    };
    publish(&state, starting.clone()).await;
    tokio::spawn(observe_workflow(state.clone(), handle, game_id));
    Ok(Json(starting))
}

async fn apply_chaos(
    State(state): State<AppState>,
    AxumPath(command): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let command = match command.as_str() {
        "double-points" => ChaosCommand::DoublePoints,
        "rust-only" => ChaosCommand::RustOnly,
        "sudden-death" => ChaosCommand::SuddenDeath,
        "extend" => ChaosCommand::ExtendThirtySeconds,
        _ => {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                format!("unknown chaos command: {command}"),
            ));
        }
    };
    if state.active_workflow.lock().await.is_none() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "no game is running".to_owned(),
        ));
    }
    let handle = state
        .client
        .get_workflow_handle::<GameWorkflowRun>(ACTIVE_WORKFLOW_ID);
    handle
        .signal(
            GameWorkflow::apply_chaos,
            command,
            WorkflowSignalOptions::default(),
        )
        .await
        .map_err(|error| ApiError(StatusCode::BAD_GATEWAY, error.to_string()))?;
    Ok(Json(serde_json::json!({ "accepted": true })))
}

async fn crash_worker() -> Result<Json<serde_json::Value>, ApiError> {
    if std::env::var("TRIVIA_SUPERVISED").as_deref() != Ok("1") {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "controller was not started with ./run-web.sh; refusing an unrecoverable exit"
                .to_owned(),
        ));
    }
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::process::exit(SUPERVISOR_RESTART_EXIT);
    });
    Ok(Json(serde_json::json!({
        "accepted": true,
        "message": "Temporal is holding the game while the Mac Worker restarts"
    })))
}

async fn round_history(State(state): State<AppState>) -> Result<Json<Vec<RoundSummary>>, ApiError> {
    let stream = state.client.list_workflows(
        "WorkflowId = 'temporal-trivia-active' AND ExecutionStatus = 'Completed'",
        WorkflowListOptions::builder()
            .limit(ROUND_HISTORY_SCAN_LIMIT)
            .build(),
    );
    tokio::pin!(stream);
    let mut rounds = Vec::new();
    while let Some(execution) = stream.next().await {
        let execution =
            execution.map_err(|error| ApiError(StatusCode::BAD_GATEWAY, error.to_string()))?;
        if let Some(summary) = round_summary(&execution) {
            rounds.push(summary);
        }
    }
    rounds.sort_by_key(|round| std::cmp::Reverse(round.closed_unix_ms.unwrap_or_default()));
    rounds.truncate(ROUND_HISTORY_LIMIT);
    Ok(Json(rounds))
}

fn round_summary(execution: &WorkflowExecution) -> Option<RoundSummary> {
    let payload = execution.memo()?.fields.get("TriviaRoundSummary")?;
    let memo: RoundMemo = serde_json::from_slice(&payload.data).ok()?;
    Some(RoundSummary {
        game_id: memo.game_id,
        run_id: execution.run_id().to_owned(),
        closed_unix_ms: execution.close_time().map(system_time_unix_ms),
        winners: memo.winners,
        badge_count: memo.badge_count,
        correct_answers: memo.correct_answers,
        wrong_answers: memo.wrong_answers,
        crashes: memo.crashes,
        reassignments: memo.reassignments,
    })
}

fn system_time_unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn resume_active_game(state: AppState) {
    tokio::spawn(async move {
        let handle = state
            .client
            .get_workflow_handle::<GameWorkflowRun>(ACTIVE_WORKFLOW_ID);
        let Ok(snapshot) = handle
            .query(GameWorkflow::snapshot, (), WorkflowQueryOptions::default())
            .await
        else {
            return;
        };
        let Some(game_id) = snapshot.game_id.clone() else {
            return;
        };
        let running = snapshot.status == GameStatus::Running;
        if running {
            *state.active_workflow.lock().await = Some(game_id.clone());
        }
        publish(&state, snapshot).await;
        if running {
            observe_workflow(state, handle, game_id).await;
        }
    });
}

async fn observe_workflow(
    state: AppState,
    handle: WorkflowHandle<Client, GameWorkflowRun>,
    game_id: String,
) {
    let mut consecutive_errors = 0_u8;
    loop {
        if state.active_workflow.lock().await.as_deref() != Some(&game_id) {
            return;
        }
        match handle
            .query(GameWorkflow::snapshot, (), WorkflowQueryOptions::default())
            .await
        {
            Ok(snapshot) => {
                consecutive_errors = 0;
                let finished = snapshot.status == GameStatus::Finished;
                publish(&state, snapshot).await;
                if finished {
                    let mut active = state.active_workflow.lock().await;
                    if active.as_deref() == Some(&game_id) {
                        *active = None;
                    }
                    return;
                }
            }
            Err(error) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors == 1 || consecutive_errors.is_multiple_of(20) {
                    eprintln!("Workflow query failed repeatedly: {error}");
                }
            }
        }
        tokio::time::sleep(observer_delay(consecutive_errors)).await;
    }
}

fn observer_delay(consecutive_errors: u8) -> Duration {
    if consecutive_errors == 0 {
        return WORKFLOW_POLL_INTERVAL;
    }
    let exponent = u32::from(consecutive_errors.saturating_sub(1).min(4));
    WORKFLOW_POLL_INTERVAL
        .saturating_mul(2_u32.pow(exponent))
        .min(WORKFLOW_MAX_ERROR_BACKOFF)
}

async fn publish(state: &AppState, snapshot: GameSnapshot) {
    *state.snapshot.write().await = snapshot.clone();
    if let Ok(json) = serde_json::to_string(&snapshot) {
        let _ = state.events.send(json);
    }
}

async fn connect_cloud() -> Result<Client> {
    let settings = read_cloud_settings()?;
    let address = required(&settings, "TEMPORAL_ADDRESS")?;
    let target = if address.contains("://") {
        address.to_owned()
    } else {
        format!("https://{address}")
    };
    let options = ConnectionOptions::new(Url::from_str(&target)?)
        .api_key(required(&settings, "TEMPORAL_API_KEY")?)
        .tls_options(TlsOptions::default())
        .build();
    let connection = Connection::connect(options).await?;
    Client::new(
        connection,
        ClientOptions::new(required(&settings, "TEMPORAL_NAMESPACE")?).build(),
    )
    .map_err(Into::into)
}

fn read_cloud_settings() -> Result<HashMap<String, String>> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project = manifest.parent().context("locate repository root")?;
    let local_path = project.join(".env.temporal");
    let legacy_path = project
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("TrafficLight/.env"));
    let path = std::env::var_os("TEMPORAL_ENV_FILE")
        .map(PathBuf::from)
        .or_else(|| local_path.is_file().then_some(local_path.clone()))
        .unwrap_or_else(|| legacy_path.unwrap_or(local_path));
    let mut settings = if path.is_file() {
        parse_env_file(&path)?
    } else {
        HashMap::new()
    };
    for key in ["TEMPORAL_ADDRESS", "TEMPORAL_NAMESPACE", "TEMPORAL_API_KEY"] {
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            settings.insert(key.to_owned(), value);
        }
    }
    Ok(settings)
}

fn parse_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read Temporal settings from {}", path.display()))?;
    temporal_trivia_shared::parse_env(&content)
        .with_context(|| format!("parse Temporal settings from {}", path.display()))
}

fn required<'a>(settings: &'a HashMap<String, String>, name: &str) -> Result<&'a str> {
    let value = settings.get(name).map(String::as_str).unwrap_or("");
    if value.is_empty() {
        bail!("missing {name}; set it in the environment or .env.temporal");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_backoff_is_bounded() {
        assert_eq!(observer_delay(0), Duration::from_millis(250));
        assert_eq!(observer_delay(1), Duration::from_millis(250));
        assert_eq!(observer_delay(2), Duration::from_millis(500));
        assert_eq!(observer_delay(20), Duration::from_secs(4));
    }
}
