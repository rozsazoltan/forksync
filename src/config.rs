use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub owner: String,
    pub max_repos_per_run: usize,
    pub concurrency: usize,
    pub min_wait_seconds: u64,
    pub max_wait_seconds: u64,
    pub grace_days: i64,
    pub schedule_time: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            owner: String::new(),
            max_repos_per_run: 50,
            concurrency: 4,
            min_wait_seconds: 10,
            max_wait_seconds: 15,
            grace_days: 7,
            schedule_time: "09:00".into(),
        }
    }
}

pub fn app_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("User configuration directory not found")?;
    Ok(base.join("forksync"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("config.toml"))
}
pub fn state_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("state.json"))
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let config: Self = toml::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("Invalid configuration: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.max_repos_per_run == 0 {
            bail!("max_repos_per_run must be at least 1");
        }
        if self.concurrency == 0 || self.concurrency > 32 {
            bail!("concurrency: 1..32");
        }
        if self.min_wait_seconds > self.max_wait_seconds || self.max_wait_seconds > 3600 {
            bail!("Wait must be 0..3600 seconds, with min <= max");
        }
        if !(1..=3650).contains(&self.grace_days) {
            bail!("grace_days: 1..3650");
        }
        parse_schedule_time(&self.schedule_time)?;
        if !self.owner.is_empty() && !valid_owner(&self.owner) {
            bail!("Invalid GitHub user or organization");
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        self.validate()?;
        let path = config_path()?;
        fs::create_dir_all(
            path.parent()
                .context("Configuration directory is missing")?,
        )?;
        let body = toml::to_string_pretty(self)?;
        let mut attempt = 0u32;
        let (temporary, mut file) = loop {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let candidate = path.with_extension(format!("toml.{}.{}.tmp", std::process::id(), id));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => break (candidate, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 10 => {
                    attempt += 1;
                }
                Err(e) => return Err(e.into()),
            }
        };
        let result = file
            .write_all(body.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let mut updated = self.clone();
        match key {
            "owner" => updated.owner = value.trim().to_owned(),
            "max_repos_per_run" => updated.max_repos_per_run = value.parse()?,
            "concurrency" => updated.concurrency = value.parse()?,
            "min_wait_seconds" => updated.min_wait_seconds = value.parse()?,
            "max_wait_seconds" => updated.max_wait_seconds = value.parse()?,
            "grace_days" => updated.grace_days = value.parse()?,
            "schedule_time" => updated.schedule_time = value.into(),
            _ => bail!("Unknown configuration key: {key}"),
        }
        updated.validate()?;
        *self = updated;
        Ok(())
    }

    pub fn unset(&mut self, key: &str) -> Result<()> {
        let default = Self::default();
        let mut updated = self.clone();
        match key {
            "owner" => updated.owner = default.owner,
            "max_repos_per_run" => updated.max_repos_per_run = default.max_repos_per_run,
            "concurrency" => updated.concurrency = default.concurrency,
            "min_wait_seconds" => updated.min_wait_seconds = default.min_wait_seconds,
            "max_wait_seconds" => updated.max_wait_seconds = default.max_wait_seconds,
            "grace_days" => updated.grace_days = default.grace_days,
            "schedule_time" => updated.schedule_time = default.schedule_time,
            _ => bail!("Unknown configuration key: {key}"),
        }
        updated.validate()?;
        *self = updated;
        Ok(())
    }
}

pub fn parse_schedule_time(time: &str) -> Result<(u8, u8)> {
    let (hour, minute) = time.split_once(':').context("schedule_time: HH:MM")?;
    if hour.len() != 2 || minute.len() != 2 {
        bail!("schedule_time: HH:MM");
    }
    let hour: u8 = hour.parse()?;
    let minute: u8 = minute.parse()?;
    if hour > 23 || minute > 59 {
        bail!("schedule_time must be a valid HH:MM time");
    }
    Ok((hour, minute))
}

pub fn valid_owner(owner: &str) -> bool {
    owner.len() <= 39
        && !owner.is_empty()
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults() {
        Config::default().validate().unwrap();
    }
    #[test]
    fn bad_wait() {
        let c = Config {
            min_wait_seconds: 16,
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }
    #[test]
    fn bad_owner() {
        assert!(!valid_owner("someone/else"));
    }
    #[test]
    fn setting_failure_does_not_mutate() {
        let mut c = Config::default();
        assert!(c.set("concurrency", "0").is_err());
        assert_eq!(c.concurrency, 4);
    }
    #[test]
    fn resetting_to_defaults() {
        let mut c = Config::default();
        c.set("grace_days", "2").unwrap();
        c.unset("grace_days").unwrap();
        assert_eq!(c.grace_days, 7);
    }
    #[test]
    fn schedule() {
        assert!(parse_schedule_time("23:59").is_ok());
        assert!(parse_schedule_time("24:00").is_err());
    }
}
