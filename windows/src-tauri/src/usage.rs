use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub project: String,
    pub agent: String,
    pub date: String, // YYYY-MM-DD
    pub tokens: u64,
    pub sessions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageHistory {
    pub entries: Vec<UsageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_tokens: u64,
    pub total_sessions: u32,
    pub total_projects: usize,
    pub total_agents: usize,
    pub breakdown: Vec<UsageBreakdownItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageBreakdownItem {
    pub project: String,
    pub agent: String,
    pub tokens: u64,
    pub sessions: u32,
}

pub struct UsageStore {
    path: Option<PathBuf>,
    history: Mutex<UsageHistory>,
}

impl UsageStore {
    pub fn new() -> Self {
        let path = dirs::config_dir().map(|d| d.join("AgentPet").join("usage_history.json"));
        let initial_history = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<UsageHistory>(&s).ok())
            .unwrap_or_default();

        Self {
            path,
            history: Mutex::new(initial_history),
        }
    }

    pub fn record_tokens(&self, project: &str, agent: &str, tokens: u64, session_count: u32) {
        if tokens == 0 && session_count == 0 {
            return;
        }
        let date = today_string();
        let proj = norm_project(project);
        let ag = agent.to_string();

        let mut hist = self.history.lock().unwrap();
        if let Some(entry) = hist
            .entries
            .iter_mut()
            .find(|e| e.project == proj && e.agent == ag && e.date == date)
        {
            entry.tokens += tokens;
            entry.sessions += session_count;
        } else {
            hist.entries.push(UsageEntry {
                project: proj,
                agent: ag,
                date,
                tokens,
                sessions: session_count,
            });
        }
        drop(hist);
        self.save();
    }

    pub fn get_summary(&self, project_filter: Option<&str>, agent_filter: Option<&str>) -> UsageSummary {
        let hist = self.history.lock().unwrap();
        let mut total_tokens = 0u64;
        let mut total_sessions = 0u32;
        let mut map: HashMap<(String, String), (u64, u32)> = HashMap::new();

        for e in &hist.entries {
            if let Some(pf) = project_filter {
                if !pf.is_empty() && pf != "all" && e.project != norm_project(pf) {
                    continue;
                }
            }
            if let Some(af) = agent_filter {
                if !af.is_empty() && af != "all" && e.agent != af {
                    continue;
                }
            }

            total_tokens += e.tokens;
            total_sessions += e.sessions;

            let key = (e.project.clone(), e.agent.clone());
            let item = map.entry(key).or_insert((0, 0));
            item.0 += e.tokens;
            item.1 += e.sessions;
        }

        let mut projects_set = std::collections::HashSet::new();
        let mut agents_set = std::collections::HashSet::new();

        let mut breakdown: Vec<UsageBreakdownItem> = map
            .into_iter()
            .map(|((project, agent), (tokens, sessions))| {
                projects_set.insert(project.clone());
                agents_set.insert(agent.clone());
                UsageBreakdownItem {
                    project,
                    agent,
                    tokens,
                    sessions,
                }
            })
            .collect();

        breakdown.sort_by(|a, b| b.tokens.cmp(&a.tokens));

        UsageSummary {
            total_tokens,
            total_sessions,
            total_projects: projects_set.len(),
            total_agents: agents_set.len(),
            breakdown,
        }
    }

    fn save(&self) {
        if let Some(ref p) = self.path {
            if let Some(dir) = p.parent() {
                let _ = fs::create_dir_all(dir);
            }
            if let Ok(hist) = self.history.lock() {
                let _ = fs::write(p, serde_json::to_string_pretty(&*hist).unwrap_or_default());
            }
        }
    }
}

fn norm_project(p: &str) -> String {
    if p.trim().is_empty() {
        "Default Project".to_string()
    } else {
        p.replace('\\', "/").trim_end_matches('/').to_string()
    }
}

fn today_string() -> String {
    // Standard ISO date YYYY-MM-DD
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = dur / 86400;
    // Approximated date from unix days
    let year = 1970 + (days / 365);
    let day_of_year = days % 365;
    let month = (day_of_year / 30) + 1;
    let day = (day_of_year % 30) + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_store() {
        let store = UsageStore::new();
        store.record_tokens("E:/project-a", "opencode", 1000, 1);
        store.record_tokens("E:/project-a", "opencode", 500, 1);
        let sum = store.get_summary(Some("E:/project-a"), None);
        assert_eq!(sum.total_tokens, 1500);
        assert_eq!(sum.total_sessions, 2);
    }
}
