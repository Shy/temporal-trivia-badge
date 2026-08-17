mod display;
mod identity;
mod input;
mod model;
mod power;
mod session;

use std::{
    convert::TryInto,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::peripherals::Peripherals,
    io::vfs::MountedEventfs,
    nvs::EspDefaultNvsPartition,
    sntp::{EspSntp, SyncStatus},
    wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};
use rustls::{RootCertStore, client::WebPkiServerVerifier};
use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, TlsOptions, WorkflowQueryOptions,
    WorkflowSignalOptions,
};
use temporalio_common::worker::{WorkerDeploymentOptions, WorkerTaskTypes};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::{
    SyncWorkflowContext, Worker, WorkerOptions, WorkflowContext, WorkflowContextView,
    WorkflowResult,
    activities::{ActivityContext, ActivityError},
};
use temporalio_sdk_core::{
    ActivitySlotKind, CoreRuntime, FixedSizeSlotSupplier, RuntimeOptions, TunerBuilder, Url,
};

use crate::{
    display::BadgeDisplay,
    identity::{BadgeIdentity, factory_identity},
    input::BadgeInput,
    model::{BADGE_TASK_QUEUE, BadgeAnswer, BadgeEvent, GameInput, GameSnapshot, QuestionTask},
    session::SessionStore,
};

const WIFI_SSID: &str = env!("BADGE_WIFI_SSID");
const WIFI_PASS: &str = env!("BADGE_WIFI_PASS");
const TEMPORAL_ADDRESS: &str = env!("TEMPORAL_ADDRESS");
const TEMPORAL_NAMESPACE: &str = env!("TEMPORAL_NAMESPACE");
const TEMPORAL_API_KEY: &str = env!("TEMPORAL_API_KEY");
const BUILD_UNIX_EPOCH: &str = env!("BADGE_BUILD_UNIX_EPOCH");
const PANIC_HOLD: Duration = Duration::from_millis(500);
const HEARTBEAT_BLACKOUT: Duration = Duration::from_secs(6);

type SharedDisplay = Arc<Mutex<BadgeDisplay>>;
type SharedInput = Arc<Mutex<BadgeInput>>;

#[workflow]
#[derive(Default)]
struct GameWorkflow;

#[workflow_methods]
impl GameWorkflow {
    #[run]
    async fn run(
        _ctx: &mut WorkflowContext<Self>,
        _input: GameInput,
    ) -> WorkflowResult<GameSnapshot> {
        unreachable!("badge firmware never registers the Workflow implementation")
    }

    #[signal]
    fn badge_started(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _event: BadgeEvent) {}

    #[signal]
    fn panic_event(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _event: BadgeEvent) {}

    #[signal]
    fn recovered(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _event: BadgeEvent) {}

    #[signal]
    fn wrong_answer(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _answer: BadgeAnswer) {}

    #[query]
    fn snapshot(&self, _ctx: &WorkflowContextView) -> GameSnapshot {
        GameSnapshot::default()
    }
}

struct BadgeActivities {
    display: SharedDisplay,
    input: SharedInput,
    identity: BadgeIdentity,
    session: Arc<SessionStore>,
    watched_game: Mutex<Option<String>>,
    activity_active: Arc<AtomicBool>,
}

struct ActivityActiveGuard(Arc<AtomicBool>);

impl ActivityActiveGuard {
    fn new(active: Arc<AtomicBool>) -> Self {
        active.store(true, Ordering::Release);
        Self(active)
    }
}

impl Drop for ActivityActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

enum Choice {
    Answer(u8),
    Panic,
}

#[activities]
impl BadgeActivities {
    #[activity(name = "trivia.answer_question")]
    async fn answer_question(
        self: Arc<Self>,
        ctx: ActivityContext,
        task: QuestionTask,
    ) -> Result<BadgeAnswer, ActivityError> {
        let _active = ActivityActiveGuard::new(Arc::clone(&self.activity_active));
        self.session
            .begin_game(&task.game_id, task.deadline_unix_ms)?;
        self.start_result_watcher(&ctx, &task)?;

        if self
            .session
            .is_abandoned(&task.game_id, &task.question.id)?
        {
            tokio::time::sleep(Duration::from_millis(250)).await;
            return Err(anyhow!("badge already abandoned this question").into());
        }

        let event = BadgeEvent {
            badge_id: self.identity.id.clone(),
            callsign: self.identity.callsign.clone(),
            question_id: task.question.id.clone(),
        };
        if let Some(handle) = ctx.workflow_handle::<GameWorkflow>() {
            if let Err(error) = handle
                .signal(
                    GameWorkflow::badge_started,
                    event.clone(),
                    WorkflowSignalOptions::default(),
                )
                .await
            {
                log::warn!("could not signal badge start: {error}");
            }
        }

        show_question(&self.display, &self.identity.callsign, &task)?;
        let activity_deadline_unix_ms = task.max_deadline_unix_ms.max(task.deadline_unix_ms);
        match self
            .wait_for_choice(&ctx, activity_deadline_unix_ms)
            .await?
        {
            Choice::Answer(selected_index) => {
                let correct = selected_index == task.question.correct_index;
                show_feedback(&self.display, &self.identity.callsign, correct)?;
                tokio::time::sleep(Duration::from_millis(350)).await;
                show_waiting(&self.display, &self.identity.callsign)?;
                let answer = BadgeAnswer {
                    badge_id: self.identity.id.clone(),
                    callsign: self.identity.callsign.clone(),
                    question_id: task.question.id.clone(),
                    selected_index,
                };
                if correct {
                    Ok(answer)
                } else {
                    self.session.abandon(&task.game_id, &task.question.id)?;
                    if let Some(handle) = ctx.workflow_handle::<GameWorkflow>() {
                        if let Err(error) = handle
                            .signal(
                                GameWorkflow::wrong_answer,
                                answer,
                                WorkflowSignalOptions::default(),
                            )
                            .await
                        {
                            log::warn!("could not signal wrong answer: {error}");
                        }
                    }
                    Err(anyhow!("incorrect answer; retry question on another badge").into())
                }
            }
            Choice::Panic => {
                self.session.abandon(&task.game_id, &task.question.id)?;
                if let Some(handle) = ctx.workflow_handle::<GameWorkflow>() {
                    if let Err(error) = handle
                        .signal(
                            GameWorkflow::panic_event,
                            event.clone(),
                            WorkflowSignalOptions::default(),
                        )
                        .await
                    {
                        log::warn!("could not signal panic: {error}");
                    }
                }
                show_panic(&self.display, &self.identity.callsign)?;
                log::warn!(
                    "simulated crash: suppressing heartbeats for {} seconds",
                    HEARTBEAT_BLACKOUT.as_secs()
                );
                // Intentionally do not heartbeat or complete. Temporal's
                // heartbeat timeout retries this Activity on another Worker.
                tokio::time::sleep(HEARTBEAT_BLACKOUT).await;
                show_recovered(&self.display, &self.identity.callsign)?;
                if let Some(handle) = ctx.workflow_handle::<GameWorkflow>() {
                    if let Err(error) = handle
                        .signal(
                            GameWorkflow::recovered,
                            event,
                            WorkflowSignalOptions::default(),
                        )
                        .await
                    {
                        log::warn!("could not signal recovery: {error}");
                    }
                }
                Err(anyhow!("simulated badge Worker crash after heartbeat timeout").into())
            }
        }
    }
}

impl BadgeActivities {
    async fn wait_for_choice(
        &self,
        ctx: &ActivityContext,
        deadline_unix_ms: u64,
    ) -> Result<Choice, ActivityError> {
        while self.sample_buttons()?.any() {
            if ctx.is_cancelled() || unix_ms() >= deadline_unix_ms {
                return Err(ActivityError::cancelled());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let mut left_armed = false;
        let mut right_armed = false;
        let mut combo_started: Option<Instant> = None;
        let mut last_heartbeat = Instant::now();
        loop {
            if ctx.is_cancelled() || unix_ms() >= deadline_unix_ms {
                return Err(ActivityError::cancelled());
            }
            if last_heartbeat.elapsed() >= Duration::from_secs(1) {
                ctx.record_heartbeat(Vec::new());
                last_heartbeat = Instant::now();
            }
            let buttons = self.sample_buttons()?;
            if buttons.left && buttons.right {
                let started = combo_started.get_or_insert_with(Instant::now);
                if started.elapsed() >= PANIC_HOLD {
                    return Ok(Choice::Panic);
                }
                left_armed = true;
                right_armed = true;
            } else {
                if combo_started.take().is_some() {
                    left_armed = false;
                    right_armed = false;
                }
                if buttons.up {
                    return Ok(Choice::Answer(0));
                }
                if buttons.down {
                    return Ok(Choice::Answer(3));
                }
                if buttons.left {
                    left_armed = true;
                } else if left_armed && !right_armed {
                    return Ok(Choice::Answer(2));
                }
                if buttons.right {
                    right_armed = true;
                } else if right_armed && !left_armed {
                    return Ok(Choice::Answer(1));
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn sample_buttons(&self) -> Result<input::Buttons, ActivityError> {
        Ok(self
            .input
            .lock()
            .map_err(|_| anyhow!("input lock poisoned"))?
            .sample())
    }

    fn start_result_watcher(
        &self,
        ctx: &ActivityContext,
        task: &QuestionTask,
    ) -> Result<(), ActivityError> {
        let mut watched = self
            .watched_game
            .lock()
            .map_err(|_| anyhow!("watcher lock poisoned"))?;
        if watched.as_deref() == Some(&task.game_id) {
            return Ok(());
        }
        *watched = Some(task.game_id.clone());
        let Some(handle) = ctx.workflow_handle::<GameWorkflow>() else {
            return Ok(());
        };
        let display = Arc::clone(&self.display);
        let identity = self.identity.clone();
        let deadline_unix_ms = task.deadline_unix_ms;
        tokio::spawn(async move {
            let wait_ms = deadline_unix_ms.saturating_sub(unix_ms());
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            for _ in 0..45 {
                match handle
                    .query(GameWorkflow::snapshot, (), WorkflowQueryOptions::default())
                    .await
                {
                    Ok(snapshot) if snapshot.status == model::GameStatus::Finished => {
                        if let Ok(mut screen) = display.lock() {
                            if let Err(error) =
                                screen.show_results(&identity.callsign, &identity.id, &snapshot)
                            {
                                log::error!("show final results: {error:#}");
                            }
                        }
                        return;
                    }
                    Ok(_) => {}
                    Err(error) => log::warn!("result query pending: {error}"),
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            if let Ok(mut screen) = display.lock() {
                let _ = screen.show_status(&identity.callsign, "RESULT PENDING");
            }
        });
        Ok(())
    }
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    validate_config()?;
    let identity = factory_identity()?;
    log::info!("Temporal Trivia badge booting as {}", identity.callsign);

    let peripherals = Peripherals::take().context("take ESP32 peripherals")?;
    let display = Arc::new(Mutex::new(BadgeDisplay::new(
        peripherals.i2c0,
        peripherals.pins.gpio4,
        peripherals.pins.gpio5,
    )?));
    let input = Arc::new(Mutex::new(BadgeInput::new(
        peripherals.pins.gpio7,
        peripherals.pins.gpio18,
        peripherals.pins.gpio17,
        peripherals.pins.gpio0,
    )?));
    show_status(&display, &identity.callsign, "BOOTING")?;

    let sys_loop = EspSystemEventLoop::take().context("take system event loop")?;
    let nvs_partition = EspDefaultNvsPartition::take().context("take default NVS")?;
    let session = Arc::new(SessionStore::new(nvs_partition.clone())?);
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs_partition))?,
        sys_loop,
    )?;
    show_status(&display, &identity.callsign, "CONNECTING WIFI")?;
    connect_wifi(&mut wifi)?;
    show_status(&display, &identity.callsign, "SYNCING TIME")?;
    let (_sntp, used_network_time) = sync_clock()?;
    if !used_network_time {
        log::warn!("using firmware build timestamp for TLS validation");
    }

    let _eventfs = MountedEventfs::mount(5).context("mount eventfd VFS for Tokio")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build single-thread Tokio runtime")?;
    runtime.block_on(Box::pin(run_worker(display, input, identity, session)))
}

async fn run_worker(
    display: SharedDisplay,
    input: SharedInput,
    identity: BadgeIdentity,
    session: Arc<SessionStore>,
) -> Result<()> {
    let runtime_options = RuntimeOptions::builder()
        .build()
        .map_err(|error| anyhow!(error))?;
    let core = CoreRuntime::new_assume_tokio(runtime_options)?;
    let target = if TEMPORAL_ADDRESS.contains("://") {
        TEMPORAL_ADDRESS.to_owned()
    } else {
        format!("https://{TEMPORAL_ADDRESS}")
    };
    show_status(&display, &identity.callsign, "CONNECTING CLOUD")?;
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|error| anyhow!("build WebPKI verifier: {error}"))?;
    let options = ConnectionOptions::new(Url::from_str(&target)?)
        .api_key(TEMPORAL_API_KEY)
        .tls_options(TlsOptions {
            server_cert_verifier: Some(verifier),
            ..Default::default()
        })
        .build();
    let connection = Connection::connect(options).await?;
    let client = Client::new(connection, ClientOptions::new(TEMPORAL_NAMESPACE).build())?;
    // A physical badge has one screen and one set of buttons, so it must never
    // execute two question Activities concurrently.
    let mut tuner = TunerBuilder::default();
    tuner.activity_slot_supplier(Arc::new(FixedSizeSlotSupplier::<ActivitySlotKind>::new(1)));
    let activity_active = Arc::new(AtomicBool::new(false));
    let worker_options = WorkerOptions::new(BADGE_TASK_QUEUE)
        .client_identity_override(identity.id.clone())
        .task_types(WorkerTaskTypes::activity_only())
        .tuner(Arc::new(tuner.build()))
        .deployment_options(WorkerDeploymentOptions::from_build_id(
            "temporal-trivia-badge-0.1.0".to_owned(),
        ))
        .register_activities(BadgeActivities {
            display: Arc::clone(&display),
            input: Arc::clone(&input),
            identity: identity.clone(),
            session,
            watched_game: Mutex::new(None),
            activity_active: Arc::clone(&activity_active),
        })
        .build();
    let mut worker =
        Worker::new(&core, client, worker_options).map_err(|error| anyhow!(error.to_string()))?;
    show_waiting(&display, &identity.callsign)?;
    let sleep_display = Arc::clone(&display);
    let sleep_input = Arc::clone(&input);
    let sleep_callsign = identity.callsign.clone();
    tokio::spawn(async move {
        if let Err(error) =
            power::monitor(sleep_display, sleep_input, activity_active, sleep_callsign).await
        {
            log::error!("sleep monitor stopped: {error:#}");
        }
    });
    log::info!("Polling trivia queue {BADGE_TASK_QUEUE} as {}", identity.id);
    worker.run().await?;
    Ok(())
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn show_status(display: &SharedDisplay, title: &str, detail: &str) -> Result<()> {
    display
        .lock()
        .map_err(|_| anyhow!("display lock poisoned"))?
        .show_status(title, detail)
}

fn show_question(display: &SharedDisplay, callsign: &str, task: &QuestionTask) -> Result<()> {
    display
        .lock()
        .map_err(|_| anyhow!("display lock poisoned"))?
        .show_question(callsign, &task.question)
}

fn show_feedback(display: &SharedDisplay, callsign: &str, correct: bool) -> Result<()> {
    display
        .lock()
        .map_err(|_| anyhow!("display lock poisoned"))?
        .show_feedback(callsign, correct, if correct { 1 } else { -1 })
}

fn show_panic(display: &SharedDisplay, callsign: &str) -> Result<()> {
    display
        .lock()
        .map_err(|_| anyhow!("display lock poisoned"))?
        .show_panic(callsign)
}

fn show_recovered(display: &SharedDisplay, callsign: &str) -> Result<()> {
    display
        .lock()
        .map_err(|_| anyhow!("display lock poisoned"))?
        .show_recovered(callsign)
}

fn show_waiting(display: &SharedDisplay, callsign: &str) -> Result<()> {
    display
        .lock()
        .map_err(|_| anyhow!("display lock poisoned"))?
        .show_waiting(callsign)
}

fn validate_config() -> Result<()> {
    for (name, value) in [
        ("BADGE_WIFI_SSID", WIFI_SSID),
        ("TEMPORAL_ADDRESS", TEMPORAL_ADDRESS),
        ("TEMPORAL_NAMESPACE", TEMPORAL_NAMESPACE),
        ("TEMPORAL_API_KEY", TEMPORAL_API_KEY),
    ] {
        if value.is_empty() {
            bail!("missing build-time setting {name}");
        }
    }
    Ok(())
}

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID
            .try_into()
            .map_err(|_| anyhow!("Wi-Fi SSID is too long"))?,
        password: WIFI_PASS
            .try_into()
            .map_err(|_| anyhow!("Wi-Fi password is too long"))?,
        auth_method: if WIFI_PASS.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        ..Default::default()
    }))?;
    wifi.start().context("start Wi-Fi")?;
    wifi.connect().context("join Wi-Fi")?;
    wifi.wait_netif_up().context("wait for DHCP")?;
    Ok(())
}

fn sync_clock() -> Result<(EspSntp<'static>, bool)> {
    let sntp = EspSntp::new_default().context("start SNTP")?;
    for _ in 0..200 {
        if sntp.get_sync_status() == SyncStatus::Completed {
            return Ok((sntp, true));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let build_epoch = BUILD_UNIX_EPOCH
        .parse::<i64>()
        .context("parse firmware build timestamp")?;
    if build_epoch < 1_700_000_000 {
        bail!("SNTP timed out and firmware build timestamp is invalid");
    }
    let timestamp = esp_idf_svc::sys::timeval {
        tv_sec: build_epoch,
        tv_usec: 0,
    };
    let result = unsafe { esp_idf_svc::sys::settimeofday(&timestamp, std::ptr::null()) };
    if result != 0 {
        bail!("SNTP timed out and settimeofday failed with code {result}");
    }
    Ok((sntp, false))
}
