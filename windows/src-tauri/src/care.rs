use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareState {
    pub health: u8,
    pub hunger: u8,
    pub happiness: u8,
    pub energy: u8,
    pub exp: u32,
    pub level: u32,
    pub streak: u32,
    pub sessions_completed: u32,
}

impl Default for CareState {
    fn default() -> Self {
        Self {
            health: 100,
            hunger: 80,
            happiness: 80,
            energy: 90,
            exp: 0,
            level: 1,
            streak: 1,
            sessions_completed: 0,
        }
    }
}

impl CareState {
    pub fn feed(&mut self) {
        self.hunger = (self.hunger + 25).min(100);
        self.health = (self.health + 5).min(100);
        self.happiness = (self.happiness + 5).min(100);
    }

    pub fn play(&mut self) {
        self.happiness = (self.happiness + 20).min(100);
        if self.energy >= 5 {
            self.energy -= 5;
        }
    }

    pub fn rest(&mut self) {
        self.energy = (self.energy + 30).min(100);
    }

    pub fn clean(&mut self) {
        self.health = (self.health + 10).min(100);
    }

    pub fn add_exp(&mut self, amount: u32) {
        self.exp += amount;
        self.level = 1 + (self.exp / 100);
    }

    pub fn on_session_done(&mut self) {
        self.sessions_completed += 1;
        self.happiness = (self.happiness + 10).min(100);
        self.add_exp(15);
    }

    pub fn decay_tick(&mut self) {
        if self.hunger > 0 {
            self.hunger -= 1;
        }
        if self.energy > 0 {
            self.energy -= 1;
        }
    }
}

pub struct CareManager {
    path: Option<PathBuf>,
    state: Mutex<CareState>,
}

impl CareManager {
    pub fn new() -> Self {
        let path = dirs::config_dir().map(|d| d.join("AgentPet").join("care.json"));
        let initial_state = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<CareState>(&s).ok())
            .unwrap_or_default();

        Self {
            path,
            state: Mutex::new(initial_state),
        }
    }

    pub fn get_state(&self) -> CareState {
        self.state.lock().unwrap().clone()
    }

    pub fn perform_action(&self, action: &str) -> CareState {
        let mut st = self.state.lock().unwrap();
        match action {
            "feed" => st.feed(),
            "play" => st.play(),
            "rest" => st.rest(),
            "clean" => st.clean(),
            _ => {}
        }
        let updated = st.clone();
        drop(st);
        self.save();
        updated
    }

    pub fn on_done(&self) -> CareState {
        let mut st = self.state.lock().unwrap();
        st.on_session_done();
        let updated = st.clone();
        drop(st);
        self.save();
        updated
    }

    fn save(&self) {
        if let Some(ref p) = self.path {
            if let Some(dir) = p.parent() {
                let _ = fs::create_dir_all(dir);
            }
            if let Ok(st) = self.state.lock() {
                let _ = fs::write(p, serde_json::to_string_pretty(&*st).unwrap_or_default());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_care_actions() {
        let mut care = CareState::default();
        care.hunger = 50;
        care.feed();
        assert_eq!(care.hunger, 75);
        care.add_exp(250);
        assert_eq!(care.level, 3);
        care.on_session_done();
        assert_eq!(care.sessions_completed, 1);
    }
}
