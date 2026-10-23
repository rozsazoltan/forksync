mod auth;
mod config;
mod github;
mod schedule;
mod state;
mod sync;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use config::Config;
use fs2::FileExt;
use inquire::{Select, Text};
use std::{fs::OpenOptions, io::IsTerminal};

#[derive(Parser)]
#[command(
    name = "forksync",
    version,
    about = "GitHub fork sync · one executable"
)]
struct Cli {
    /// Never ask questions (for scheduled runs or scripts).
    #[arg(long, global = true)]
    non_interactive: bool,
    /// Print only finished repo rows, without live progress indicators.
    #[arg(long, global = true)]
    no_progress: bool,
    /// Output a machine-readable sync summary or configuration query.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Sync eligible repositories (only OK and FAILED consume the run limit).
    Sync {
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        scheduled: bool,
    },
    /// Read or update settings.
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
    },
    /// GitHub login/logout using secure credential storage.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Install/remove the operating system's daily timer.
    Schedule {
        #[command(subcommand)]
        command: ScheduleCommand,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    List,
    Get { key: String },
    Set { key: String, value: String },
    Unset { key: String },
    Path,
}
#[derive(Subcommand)]
enum AuthCommand {
    Login,
    Logout,
    Status,
}
#[derive(Subcommand)]
enum ScheduleCommand {
    Install,
    Status,
    Remove,
}

#[tokio::main]
async fn main() {
    if let Err(err) = execute().await {
        eprintln!("[FAILED] {err:#}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::load()?;
    let non_interactive = cli.non_interactive || cli.json;
    if cli.json
        && !matches!(
            &cli.command,
            Some(Command::Sync { .. })
                | Some(Command::Config {
                    command: Some(ConfigCommand::List | ConfigCommand::Get { .. })
                })
        )
    {
        bail!("--json is available only for sync, config list, and config get");
    }
    match cli.command {
        None if non_interactive => bail!("Provide a command in non-interactive mode"),
        None => menu(&mut cfg).await?,
        Some(Command::Sync { owner, scheduled }) => {
            if let Some(owner) = owner {
                cfg.owner = owner;
                cfg.validate()?;
            }
            sync_now(
                &mut cfg,
                !scheduled && !non_interactive && std::io::stdin().is_terminal(),
                scheduled,
                cli.no_progress,
                cli.json,
            )
            .await?;
        }
        Some(Command::Config { command }) => match command {
            None if non_interactive => {
                bail!("Use forksync config list|get|set|unset in non-interactive mode")
            }
            None => configure(&mut cfg)?,
            Some(ConfigCommand::List) if cli.json => println!("{}", serde_json::to_string(&cfg)?),
            Some(ConfigCommand::List) => print!("{}", toml::to_string_pretty(&cfg)?),
            Some(ConfigCommand::Path) => println!("{}", config::config_path()?.display()),
            Some(ConfigCommand::Get { key }) => {
                let repr = toml::Value::try_from(&cfg)?;
                let value = repr
                    .get(&key)
                    .ok_or_else(|| anyhow::anyhow!("Unknown key: {key}"))?;
                if cli.json {
                    let mut data = serde_json::Map::new();
                    data.insert(key, toml_to_json(value)?);
                    println!("{}", serde_json::Value::Object(data));
                } else {
                    println!("{}", value);
                }
            }
            Some(ConfigCommand::Unset { key }) => {
                cfg.unset(&key)?;
                cfg.save()?;
                println!("Reset to default: {key}");
            }
            Some(ConfigCommand::Set { key, value }) => {
                cfg.set(&key, &value)?;
                cfg.save()?;
                println!("Saved: {key}");
            }
        },
        Some(Command::Auth { command }) => match command {
            AuthCommand::Login => {
                if non_interactive {
                    bail!("Auth login requires a terminal; use GH_TOKEN for scheduled runs");
                }
                let secret = auth::login()?;
                let viewer = github::Github::new(&secret)?.viewer().await?;
                auth::store(&secret)?;
                println!("Signed in to GitHub as {viewer}");
            }
            AuthCommand::Logout => {
                auth::logout()?;
                println!("Stored token removed. GH_TOKEN and GITHUB_TOKEN were not changed.");
            }
            AuthCommand::Status => match auth::token()? {
                Some(secret) => println!(
                    "Signed in as {}",
                    github::Github::new(&secret)?.viewer().await?
                ),
                None => println!("No token found. Run: forksync auth login"),
            },
        },
        Some(Command::Schedule { command }) => match command {
            ScheduleCommand::Install => schedule::install(&cfg)?,
            ScheduleCommand::Status => schedule::status()?,
            ScheduleCommand::Remove => schedule::remove()?,
        },
    }
    Ok(())
}

async fn menu(cfg: &mut Config) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("The interactive menu requires a terminal. Use forksync sync instead.");
    }
    loop {
        println!("\nforksync · v{}", env!("CARGO_PKG_VERSION"));
        println!();
        let option = Select::new(
            "Choose an action",
            vec![
                "Sync repositories",
                "Settings",
                "Sign in to GitHub",
                "Install daily schedule",
                "Remove daily schedule",
                "Quit",
            ],
        )
        .prompt()?;
        match option {
            "Sync repositories" => sync_now(cfg, true, false, false, false).await?,
            "Settings" => configure(cfg)?,
            "Sign in to GitHub" => {
                let secret = auth::login()?;
                let viewer = github::Github::new(&secret)?.viewer().await?;
                auth::store(&secret)?;
                println!("Signed in as {viewer}");
            }
            "Install daily schedule" => schedule::install(cfg)?,
            "Remove daily schedule" => schedule::remove()?,
            _ => return Ok(()),
        }
    }
}

fn configure(cfg: &mut Config) -> Result<()> {
    loop {
        println!("\n{}", toml::to_string_pretty(&*cfg)?);
        let opts = vec![
            "owner",
            "max_repos_per_run",
            "concurrency",
            "min_wait_seconds",
            "max_wait_seconds",
            "grace_days",
            "schedule_time",
            "Back",
        ];
        println!();
        let key = Select::new("Choose a setting to change", opts).prompt()?;
        if key == "Back" {
            return Ok(());
        }
        let current = toml::Value::try_from(&*cfg)?
            .get(key)
            .map(ToString::to_string)
            .unwrap_or_default();
        println!();
        let input = Text::new(&format!("{key} (current: {current})")).prompt()?;
        let mut updated = cfg.clone();
        match updated.set(key, &input) {
            Ok(()) => {
                updated.save()?;
                *cfg = updated;
            }
            Err(e) => eprintln!("Invalid setting: {e}"),
        }
    }
}

fn toml_to_json(value: &toml::Value) -> Result<serde_json::Value> {
    serde_json::to_value(value).map_err(Into::into)
}

async fn sync_now(
    cfg: &mut Config,
    interactive: bool,
    scheduled: bool,
    no_progress: bool,
    json: bool,
) -> Result<()> {
    if cfg.owner.is_empty() {
        if !interactive {
            bail!("Owner is missing. Set it with: forksync config set owner NAME");
        }
        println!();
        cfg.owner = Text::new("GitHub user or organization:")
            .prompt()?
            .trim()
            .to_owned();
        cfg.save()?;
    }
    cfg.validate()?;
    let available_token = match auth::token() {
        Ok(value) => value,
        Err(err) if interactive => {
            eprintln!("Credential store: {err}");
            None
        }
        Err(err) => return Err(err),
    };
    let secret = match available_token {
        Some(token) => token,
        None if interactive => {
            let token = auth::login()?;
            github::Github::new(&token)?.viewer().await?;
            if let Err(err) = auth::store(&token) {
                eprintln!("Warning: could not save token ({err}); it remains available for this run. Scheduled runs require GH_TOKEN.");
            }
            println!();
            token
        },
        None => bail!("No token found. Sign in with forksync auth login, or configure the credential store or GH_TOKEN for automated runs."),
    };
    let client = github::Github::new(&secret)?;
    let viewer = client.viewer().await?;
    // Lock is held for the entire run to prevent overlapping timers/manual runs.
    std::fs::create_dir_all(config::app_dir()?)?;
    let lockfile = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(config::app_dir()?.join("forksync.lock"))?;
    lockfile
        .try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("Another forksync run is already active"))?;
    let mut state = state::State::load()?;
    if scheduled && state.scheduled_today(&cfg.owner) {
        if json {
            println!(
                "{}",
                serde_json::json!({"status": "skipped", "reason": "already_scheduled_today", "owner": cfg.owner})
            );
        } else {
            println!(
                "[SKIP] Scheduled run already completed today: {}",
                cfg.owner
            );
        }
        return Ok(());
    }
    let forks = client.forks(&cfg.owner, &viewer).await?;
    let summary = sync::run(client, cfg, &mut state, forks, no_progress, json).await?;
    if scheduled {
        state.mark_scheduled(&cfg.owner);
    }
    state.save()?;
    if json {
        println!("{}", serde_json::to_string(&summary)?);
    }
    if summary.failed > 0 {
        bail!("{} repository sync operation(s) failed", summary.failed);
    }
    Ok(())
}
