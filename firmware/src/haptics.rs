use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use esp_idf_svc::hal::ledc::LedcDriver;
use tokio::sync::Mutex;

const ORIGINAL_STRENGTH: u8 = 155;
const SOFT_STRENGTH: u8 = 110;
const FIRM_STRENGTH: u8 = 200;
const ORIGINAL_PULSE: Duration = Duration::from_millis(35);
const PATTERN_GAP: Duration = Duration::from_millis(80);

pub type SharedHaptics = Arc<Mutex<BadgeHaptics>>;

#[derive(Clone, Copy, Debug)]
pub enum HapticEvent {
    SleepCountdown,
    Correct,
    Wrong,
    Crash,
    Recovered,
    Winner,
    RoundOver,
}

pub struct BadgeHaptics {
    driver: LedcDriver<'static>,
}

struct MotorOffGuard<'a> {
    driver: &'a mut LedcDriver<'static>,
    armed: bool,
}

impl MotorOffGuard<'_> {
    fn stop(mut self) -> Result<()> {
        self.driver.set_duty(0).context("stop haptic motor")?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for MotorOffGuard<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = self.driver.set_duty(0)
        {
            log::error!("emergency haptic motor stop failed: {error}");
        }
    }
}

impl BadgeHaptics {
    pub fn new(mut driver: LedcDriver<'static>) -> Result<Self> {
        driver.set_duty(0).context("turn haptic motor off")?;
        Ok(Self { driver })
    }

    async fn pulse(&mut self, strength: u8, duration: Duration) -> Result<()> {
        let duty = self.driver.get_max_duty() * u32::from(strength) / u32::from(u8::MAX);
        self.driver.set_duty(duty).context("start haptic pulse")?;
        let guard = MotorOffGuard {
            driver: &mut self.driver,
            armed: true,
        };
        tokio::time::sleep(duration).await;
        guard.stop()
    }

    async fn gap(&self) {
        tokio::time::sleep(PATTERN_GAP).await;
    }

    async fn play(&mut self, event: HapticEvent) -> Result<()> {
        match event {
            HapticEvent::SleepCountdown | HapticEvent::Correct | HapticEvent::RoundOver => {
                self.pulse(ORIGINAL_STRENGTH, ORIGINAL_PULSE).await?;
            }
            HapticEvent::Wrong => {
                self.pulse(SOFT_STRENGTH, ORIGINAL_PULSE).await?;
                self.gap().await;
                self.pulse(SOFT_STRENGTH, ORIGINAL_PULSE).await?;
            }
            HapticEvent::Crash => {
                self.pulse(FIRM_STRENGTH, Duration::from_millis(120))
                    .await?;
            }
            HapticEvent::Recovered => {
                self.pulse(ORIGINAL_STRENGTH, ORIGINAL_PULSE).await?;
                self.gap().await;
                self.pulse(ORIGINAL_STRENGTH, ORIGINAL_PULSE).await?;
            }
            HapticEvent::Winner => {
                for (index, strength) in [SOFT_STRENGTH, 135, ORIGINAL_STRENGTH]
                    .into_iter()
                    .enumerate()
                {
                    self.pulse(strength, ORIGINAL_PULSE).await?;
                    if index < 2 {
                        self.gap().await;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn off(&mut self) -> Result<()> {
        self.driver.set_duty(0).context("stop haptic motor")
    }
}

pub async fn play(haptics: &SharedHaptics, event: HapticEvent) {
    if let Err(error) = haptics.lock().await.play(event).await {
        log::error!("haptic {event:?} failed: {error:#}");
    }
}

pub async fn off(haptics: &SharedHaptics) -> Result<()> {
    haptics.lock().await.off()
}
