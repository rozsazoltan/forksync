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
    about = "GitHub fork sync · egyetlen executable"
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
        bail!("--json csak a sync, config list és config get parancsokhoz használható");
    }
    match cli.command {
        None if non_interactive => bail!("Nem interaktív módban adj meg egy alparancsot"),
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
                bail!("Nem interaktív módban használd: forksync config list|get|set|unset")
            }
            None => configure(&mut cfg)?,
            Some(ConfigCommand::List) if cli.json => println!("{}", serde_json::to_string(&cfg)?),
            Some(ConfigCommand::List) => print!("{}", toml::to_string_pretty(&cfg)?),
            Some(ConfigCommand::Path) => println!("{}", config::config_path()?.display()),
            Some(ConfigCommand::Get { key }) => {
                let repr = toml::Value::try_from(&cfg)?;
                let value = repr
                    .get(&key)
                    .ok_or_else(|| anyhow::anyhow!("Ismeretlen kulcs: {key}"))?;
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
                println!("Alapérték visszaállítva: {key}");
            }
            Some(ConfigCommand::Set { key, value }) => {
                cfg.set(&key, &value)?;
                cfg.save()?;
                println!("Mentve: {key}");
            }
        },
        Some(Command::Auth { command }) => {
            match command {
                AuthCommand::Login => {
                    if non_interactive {
                        bail!("Az auth login terminált igényel; időzítéshez használj GH_TOKEN-t");
                    }
                    let secret = auth::login()?;
                    let viewer = github::Github::new(&secret)?.viewer().await?;
                    auth::store(&secret)?;
                    println!("Sikeres GitHub bejelentkezés: {viewer}");
                }
                AuthCommand::Logout => {
                    auth::logout()?;
                    println!("A mentett token törölve. A GH_TOKEN / GITHUB_TOKEN változók nem változtak.");
                }
                AuthCommand::Status => match auth::token()? {
                    Some(secret) => println!(
                        "Bejelentkezve: {}",
                        github::Github::new(&secret)?.viewer().await?
                    ),
                    None => println!("Nincs token. Futtasd: forksync auth login"),
                },
            }
        }
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
        bail!("Interaktív menühöz terminál szükséges. Használd a forksync sync parancsot.");
    }
    loop {
        println!("\nforksync · v{}", env!("CARGO_PKG_VERSION"));
        let option = Select::new(
            "Művelet",
            vec![
                "Szinkronizálás",
                "Beállítások",
                "GitHub bejelentkezés",
                "Napi időzítés bekapcsolása",
                "Napi időzítés eltávolítása",
                "Kilépés",
            ],
        )
        .prompt()?;
        match option {
            "Szinkronizálás" => sync_now(cfg, true, false, false, false).await?,
            "Beállítások" => configure(cfg)?,
            "GitHub bejelentkezés" => {
                let secret = auth::login()?;
                let viewer = github::Github::new(&secret)?.viewer().await?;
                auth::store(&secret)?;
                println!("Bejelentkezve: {viewer}");
            }
            "Napi időzítés bekapcsolása" => schedule::install(cfg)?,
            "Napi időzítés eltávolítása" => schedule::remove()?,
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
            "Vissza",
        ];
        let key = Select::new("Melyik beállítást módosítod?", opts).prompt()?;
        if key == "Vissza" {
            return Ok(());
        }
        let current = toml::Value::try_from(&*cfg)?
            .get(key)
            .map(ToString::to_string)
            .unwrap_or_default();
        let input = Text::new(&format!("{key} (jelenleg: {current})")).prompt()?;
        let mut updated = cfg.clone();
        match updated.set(key, &input) {
            Ok(()) => {
                updated.save()?;
                *cfg = updated;
            }
            Err(e) => eprintln!("Érvénytelen beállítás: {e}"),
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
            bail!("Hiányzik az owner. Beállítás: forksync config set owner NÉV");
        }
        cfg.owner = Text::new("GitHub felhasználó vagy szervezet:")
            .prompt()?
            .trim()
            .to_owned();
        cfg.save()?;
    }
    cfg.validate()?;
    let available_token = match auth::token() {
        Ok(value) => value,
        Err(err) if interactive => {
            eprintln!("Tokentároló: {err}");
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
                eprintln!("Figyelem: a token nem menthető ({err}); erre a futásra továbbra is használható. Időzítéshez GH_TOKEN szükséges.");
            }
            token
        },
        None => bail!("Nincs token. Interaktív belépés: forksync auth login; automatikus futáshoz a felhasználói hitelesítőadat-tároló vagy GH_TOKEN szükséges."),
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
        .map_err(|_| anyhow::anyhow!("Egy másik forksync futás már aktív"))?;
    let mut state = state::State::load()?;
    if scheduled && state.scheduled_today(&cfg.owner) {
        if json {
            println!(
                "{}",
                serde_json::json!({"status": "skipped", "reason": "already_scheduled_today", "owner": cfg.owner})
            );
        } else {
            println!(
                "[SKIP] A napi ütemezett futás ma már megtörtént: {}",
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
        bail!("{} repository szinkronizálása sikertelen", summary.failed);
    }
    Ok(())
}
