mod model;
mod questions;
mod workflow;

use std::{
    collections::HashMap,
    convert::Infallible,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, TlsOptions, WorkflowHandle,
    WorkflowQueryOptions, WorkflowStartOptions,
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
    model::{GAME_SECONDS, GameInput, GameSnapshot, GameStatus, WEB_TASK_QUEUE},
    workflow::{GameWorkflow, GameWorkflowRun},
};

const INDEX_HTML: &str = include_str!("../static/index.html");
const ACTIVE_WORKFLOW_ID: &str = "temporal-trivia-active";

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
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn start_game(
    State(state): State<AppState>,
    Json(request): Json<StartRequest>,
) -> Result<Json<GameSnapshot>, ApiError> {
    if request.backlog_override == Some(0) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "backlog override must be positive".to_owned(),
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
                if consecutive_errors == 20 {
                    eprintln!("Workflow query failed repeatedly: {error}");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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
    let temporal_root = manifest
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("locate Temporal workspace")?;
    let path = temporal_root.join("TrafficLight/.env");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("read Temporal settings from {}", path.display()))?;
    Ok(content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((
                key.trim().to_owned(),
                value
                    .trim()
                    .trim_matches(|character| character == '\'' || character == '"')
                    .to_owned(),
            ))
        })
        .collect())
}

fn required<'a>(settings: &'a HashMap<String, String>, name: &str) -> Result<&'a str> {
    let value = settings.get(name).map(String::as_str).unwrap_or("");
    if value.is_empty() {
        bail!("missing {name} in TrafficLight/.env");
    }
    Ok(value)
}
