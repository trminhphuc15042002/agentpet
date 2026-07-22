use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakConfig {
    pub enabled: bool,
    pub work_interval_secs: u64,
    pub break_duration_secs: u64,
}

impl Default for BreakConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            work_interval_secs: 45 * 60,
            break_duration_secs: 5 * 60,
        }
    }
}

pub struct BreakClock {
    active_accumulated_secs: AtomicU64,
    is_active: AtomicBool,
}

impl BreakClock {
    pub fn new() -> Self {
        Self {
            active_accumulated_secs: AtomicU64::new(0),
            is_active: AtomicBool::new(false),
        }
    }

    pub fn set_active(&self, active: bool) {
        self.is_active.store(active, Ordering::SeqCst);
    }

    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::SeqCst)
    }

    pub fn tick(&self) -> u64 {
        if self.is_active.load(Ordering::SeqCst) {
            self.active_accumulated_secs.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            self.active_accumulated_secs.load(Ordering::SeqCst)
        }
    }

    pub fn reset(&self) {
        self.active_accumulated_secs.store(0, Ordering::SeqCst);
    }

    pub fn get_accumulated(&self) -> u64 {
        self.active_accumulated_secs.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_break_clock_tick() {
        let clock = BreakClock::new();
        clock.set_active(true);
        assert_eq!(clock.tick(), 1);
        assert_eq!(clock.tick(), 2);
        clock.set_active(false);
        assert_eq!(clock.tick(), 2);
        clock.reset();
        assert_eq!(clock.tick(), 0);
    }
}
