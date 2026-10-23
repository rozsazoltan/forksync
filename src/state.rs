use crate::config;
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    // Last successful check, including "already up to date" responses.
    pub checked_at: HashMap<String, DateTime<Utc>>,
    pub last_scheduled_run: HashMap<String, String>,
}

impl State {
    pub fn load() -> Result<Self> {
        let path = config::state_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }
    pub fn save(&self) -> Result<()> {
        let path = config::state_path()?;
        fs::create_dir_all(path.parent().expect("state path has parent"))?;
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        // Windows rename does not replace existing destinations; remove then rename.
        // A process lock ensures a second forksync instance cannot race this write.
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }
    pub fn due(&self, name: &str, days: i64) -> bool {
        match self.checked_at.get(name) {
            None => true,
            Some(when) => Utc::now().signed_duration_since(*when).num_seconds() >= days * 86_400,
        }
    }
    pub fn mark_checked(&mut self, name: &str) {
        self.checked_at.insert(name.to_owned(), Utc::now());
    }
    pub fn clear_checked(&mut self, name: &str) {
        self.checked_at.remove(name);
    }
    pub fn scheduled_today(&self, owner: &str) -> bool {
        self.last_scheduled_run.get(owner).map(String::as_str)
            == Some(&Local::now().format("%Y-%m-%d").to_string())
    }
    pub fn mark_scheduled(&mut self, owner: &str) {
        self.last_scheduled_run
            .insert(owner.into(), Local::now().format("%Y-%m-%d").to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unseen_is_due() {
        assert!(State::default().due("someone/repo", 7));
    }
    #[test]
    fn checked_is_not_due() {
        let mut s = State::default();
        s.mark_checked("someone/repo");
        assert!(!s.due("someone/repo", 7));
    }
}
