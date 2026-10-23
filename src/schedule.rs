use crate::config::Config;
use anyhow::{bail, Context, Result};
use std::{fs, path::PathBuf, process::Command};

#[allow(dead_code)]
const JOB: &str = "forksync";

fn exe() -> Result<PathBuf> {
    std::env::current_exe().context("Could not read executable path")
}
fn invoke(command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .context("Could not start scheduler command")?;
    if !output.status.success() {
        bail!(
            "Scheduler error: {} {}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub fn install(cfg: &Config) -> Result<()> {
    cfg.validate()?;
    if cfg.owner.is_empty() {
        bail!("Set owner first: forksync config set owner NAME");
    }
    let (hour, minute) = crate::config::parse_schedule_time(&cfg.schedule_time)?;
    let binary = exe()?;
    #[cfg(target_os = "windows")]
    {
        let task = format!("\"{}\" sync --scheduled", binary.display());
        invoke(Command::new("schtasks").args([
            "/Create",
            "/F",
            "/SC",
            "DAILY",
            "/ST",
            &cfg.schedule_time,
            "/TN",
            JOB,
            "/TR",
            &task,
        ]))?;
    }
    #[cfg(target_os = "linux")]
    {
        let dir = dirs::config_dir()
            .context("Configuration directory is missing")?
            .join("systemd/user");
        fs::create_dir_all(&dir)?;
        let escaped_path = binary
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%");
        let exec = format!("\"{escaped_path}\" sync --scheduled");
        fs::write(dir.join("forksync.service"), format!("[Unit]\nDescription=Synchronize GitHub forks\n[Service]\nType=oneshot\nExecStart={exec}\n"))?;
        fs::write(dir.join("forksync.timer"), format!("[Unit]\nDescription=Daily GitHub fork sync\n[Timer]\nOnCalendar=*-*-* {hour:02}:{minute:02}:00\nPersistent=true\nUnit=forksync.service\n[Install]\nWantedBy=timers.target\n"))?;
        invoke(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        invoke(Command::new("systemctl").args(["--user", "enable", "--now", "forksync.timer"]))?;
    }
    #[cfg(target_os = "macos")]
    {
        let dir = dirs::home_dir()
            .context("Home directory is missing")?
            .join("Library/LaunchAgents");
        fs::create_dir_all(&dir)?;
        let file = dir.join("dev.forksync.daily.plist");
        let escaped = xml_escape(&binary.to_string_lossy());
        let plist = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>dev.forksync.daily</string>\n<key>ProgramArguments</key><array><string>{escaped}</string><string>sync</string><string>--scheduled</string></array>\n<key>StartCalendarInterval</key><dict><key>Hour</key><integer>{hour}</integer><key>Minute</key><integer>{minute}</integer></dict>\n<key>RunAtLoad</key><false/>\n</dict></plist>\n");
        // Updating a loaded agent requires bootout before bootstrap.
        let domain = launchd_domain()?;
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{domain}/dev.forksync.daily")])
            .output();
        fs::write(&file, plist)?;
        invoke(
            Command::new("launchctl")
                .arg("bootstrap")
                .arg(&domain)
                .arg(&file),
        )?;
    }
    println!(
        "Daily schedule installed: {} (local time), {}",
        cfg.schedule_time,
        binary.display()
    );
    println!("Scheduler uses the current executable path. Run schedule install again after moving the executable.");
    Ok(())
}

pub fn remove() -> Result<()> {
    #[cfg(target_os = "windows")]
    invoke(Command::new("schtasks").args(["/Delete", "/TN", JOB, "/F"]))?;
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "forksync.timer"])
            .status();
        let dir = dirs::config_dir()
            .context("Configuration directory is missing")?
            .join("systemd/user");
        for name in ["forksync.timer", "forksync.service"] {
            let p = dir.join(name);
            if p.exists() {
                fs::remove_file(p)?;
            }
        }
        invoke(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    }
    #[cfg(target_os = "macos")]
    {
        let domain = launchd_domain()?;
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{domain}/dev.forksync.daily")])
            .status();
        let file = dirs::home_dir()
            .context("Home directory is missing")?
            .join("Library/LaunchAgents/dev.forksync.daily.plist");
        if file.exists() {
            fs::remove_file(file)?;
        }
    }
    println!("Daily schedule removed.");
    Ok(())
}

pub fn status() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let output = Command::new("schtasks")
            .args(["/Query", "/TN", JOB])
            .output()
            .context("Could not start scheduler command")?;
        if output.status.success() {
            println!("Schedule: installed ({JOB})");
            print!("{}", String::from_utf8_lossy(&output.stdout));
        } else {
            println!("Schedule: not installed");
        }
    }
    #[cfg(target_os = "linux")]
    {
        let dir = dirs::config_dir()
            .context("Configuration directory is missing")?
            .join("systemd/user");
        let service = dir.join("forksync.service");
        let timer = dir.join("forksync.timer");
        if service.exists() && timer.exists() {
            println!("Schedule: installed (systemd user timer)");
            let _ = Command::new("systemctl")
                .args(["--user", "is-active", "forksync.timer"])
                .status();
        } else {
            println!("Schedule: not installed");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let file = dirs::home_dir()
            .context("Home directory is missing")?
            .join("Library/LaunchAgents/dev.forksync.daily.plist");
        if file.exists() {
            println!("Schedule: installed (launchd LaunchAgent)");
        } else {
            println!("Schedule: not installed");
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn launchd_domain() -> Result<String> {
    let output = Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        bail!("Could not read user ID");
    }
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
