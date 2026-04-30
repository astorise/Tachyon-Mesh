use std::{
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};

use sysinfo::{ProcessesToUpdate, System};
use tokio::time::MissedTickBehavior;

const DEFAULT_SOFT_PERCENT: u8 = 75;
const DEFAULT_HARD_PERCENT: u8 = 90;
const GOVERNOR_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MemoryPressure {
    #[default]
    Normal = 0,
    High = 1,
    Critical = 2,
}

#[derive(Debug)]
pub(crate) struct MemoryGovernor {
    pressure: AtomicU8,
    rss_bytes: AtomicU64,
    soft_limit_bytes: u64,
    hard_limit_bytes: u64,
}

impl MemoryGovernor {
    pub(crate) fn new(total_memory_bytes: u64, soft_percent: u8, hard_percent: u8) -> Self {
        let soft_percent = soft_percent.clamp(1, 100);
        let hard_percent = hard_percent.clamp(soft_percent, 100);
        Self {
            pressure: AtomicU8::new(MemoryPressure::Normal as u8),
            rss_bytes: AtomicU64::new(0),
            soft_limit_bytes: percent_of(total_memory_bytes, soft_percent),
            hard_limit_bytes: percent_of(total_memory_bytes, hard_percent),
        }
    }

    pub(crate) fn from_system_memory() -> Self {
        let mut system = System::new();
        system.refresh_memory();
        Self::new(
            system.total_memory(),
            DEFAULT_SOFT_PERCENT,
            DEFAULT_HARD_PERCENT,
        )
    }

    pub(crate) fn pressure(&self) -> MemoryPressure {
        pressure_from_u8(self.pressure.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    pub(crate) fn rss_bytes(&self) -> u64 {
        self.rss_bytes.load(Ordering::Relaxed)
    }

    pub(crate) fn sample_rss_bytes(&self, rss_bytes: u64) -> MemoryPressure {
        self.rss_bytes.store(rss_bytes, Ordering::Relaxed);
        let pressure = classify_pressure(rss_bytes, self.soft_limit_bytes, self.hard_limit_bytes);
        self.pressure.store(pressure as u8, Ordering::Relaxed);
        pressure
    }
}

pub(crate) fn process_rss_bytes(system: &mut System) -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(|process| process.memory())
}

pub(crate) fn spawn_memory_governor(
    governor: Arc<MemoryGovernor>,
    on_pressure: impl Fn(MemoryPressure) + Send + Sync + 'static,
) {
    let on_pressure = Arc::new(on_pressure);
    tokio::spawn(async move {
        let mut system = System::new();
        let mut interval = tokio::time::interval(GOVERNOR_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Some(rss_bytes) = process_rss_bytes(&mut system) {
                let pressure = governor.sample_rss_bytes(rss_bytes);
                if pressure != MemoryPressure::Normal {
                    on_pressure(pressure);
                }
            }
        }
    });
}

fn classify_pressure(
    rss_bytes: u64,
    soft_limit_bytes: u64,
    hard_limit_bytes: u64,
) -> MemoryPressure {
    if rss_bytes >= hard_limit_bytes {
        MemoryPressure::Critical
    } else if rss_bytes >= soft_limit_bytes {
        MemoryPressure::High
    } else {
        MemoryPressure::Normal
    }
}

fn percent_of(total: u64, percent: u8) -> u64 {
    total.saturating_mul(percent as u64) / 100
}

fn pressure_from_u8(value: u8) -> MemoryPressure {
    match value {
        2 => MemoryPressure::Critical,
        1 => MemoryPressure::High,
        _ => MemoryPressure::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_memory_pressure_from_thresholds() {
        let governor = MemoryGovernor::new(1_000, 75, 90);

        assert_eq!(governor.sample_rss_bytes(749), MemoryPressure::Normal);
        assert_eq!(governor.sample_rss_bytes(750), MemoryPressure::High);
        assert_eq!(governor.sample_rss_bytes(900), MemoryPressure::Critical);
        assert_eq!(governor.rss_bytes(), 900);
    }
}
