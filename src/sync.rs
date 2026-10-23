use crate::{
    config::Config,
    github::{Github, Merge, Repo},
    state::State,
};
use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rand::Rng;
use std::{
    collections::VecDeque,
    io::IsTerminal,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, task::JoinSet};

#[derive(Default, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Summary {
    pub ok: usize,
    pub failed: usize,
    pub skipped: usize,
}
impl Summary {
    fn counted(&self) -> usize {
        self.ok + self.failed
    }
}

#[derive(Default)]
struct Pacer {
    last: Mutex<Option<Instant>>,
}
impl Pacer {
    async fn wait(&self, min: u64, max: u64) {
        let mut guard = self.last.lock().await;
        if let Some(last) = *guard {
            let delay = rand::thread_rng().gen_range(min..=max);
            let deadline = last + Duration::from_secs(delay);
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                tokio::time::sleep(wait).await;
            }
        }
        *guard = Some(Instant::now());
    }
}

#[derive(Debug)]
enum Outcome {
    Ok,
    Skip,
    Failed(String),
}

pub async fn run(
    client: Github,
    cfg: &Config,
    state: &mut State,
    repos: Vec<Repo>,
    no_progress: bool,
    json: bool,
) -> Result<Summary> {
    let total = repos.len();
    let name_width = repos
        .iter()
        .map(|repo| repo.full_name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let multiprogress = MultiProgress::new();
    let tty =
        std::io::stdout().is_terminal() && std::io::stderr().is_terminal() && !no_progress && !json;
    let style = ProgressStyle::with_template("{spinner:.cyan} {msg}")?;
    let overall = if tty && total > 0 {
        let bar = multiprogress.add(ProgressBar::new(total as u64));
        bar.set_style(ProgressStyle::with_template(
            "{spinner:.cyan} {wide_bar:.cyan/blue} {pos}/{len} repositories · {msg}",
        )?);
        bar.set_message("Preparing sync");
        Some(bar)
    } else {
        None
    };
    let pacer = Arc::new(Pacer::default());
    let mut queue: VecDeque<(usize, Repo)> = repos.into_iter().enumerate().collect();
    let mut running = JoinSet::new();
    let mut summary = Summary::default();

    if !json {
        println!(
            "forksync · syncing up to {} repositories · {} workers · {}-day cooldown",
            cfg.max_repos_per_run, cfg.concurrency, cfg.grace_days
        );
        if tty {
            println!("  {:<name_width$}  RESULT", "REPOSITORY");
            println!("  {}", "─".repeat(name_width + 18));
        }
    }

    while !queue.is_empty() || !running.is_empty() {
        while running.len() < cfg.concurrency
            && summary.counted() + running.len() < cfg.max_repos_per_run
        {
            let Some((index, repo)) = queue.pop_front() else {
                break;
            };
            let prefix = repo_prefix(name_width, &repo.full_name);
            let counter = repo_counter(index, total);
            let bar = multiprogress.add(ProgressBar::new_spinner());
            bar.set_style(style.clone());
            bar.set_message(format!("{prefix} ..."));
            if repo.archived || repo.disabled || repo.default_branch.is_empty() {
                finish_line(
                    &bar,
                    format!(
                        "{prefix} {} Archived, disabled, or empty repository{counter}",
                        status("SKIP", "yellow", tty)
                    ),
                    tty,
                    json,
                );
                summary.skipped += 1;
                if let Some(overall) = &overall {
                    overall.inc(1);
                }
                continue;
            }
            if !state.due(&repo.full_name, cfg.grace_days) {
                finish_line(
                    &bar,
                    format!(
                        "{prefix} {} Cooldown active{counter}",
                        status("SKIP", "yellow", tty)
                    ),
                    tty,
                    json,
                );
                summary.skipped += 1;
                if let Some(overall) = &overall {
                    overall.inc(1);
                }
                continue;
            }
            let client = client.clone();
            let pacer = pacer.clone();
            let min = cfg.min_wait_seconds;
            let max = cfg.max_wait_seconds;
            running.spawn(async move {
                pacer.wait(min, max).await;
                let outcome = match client.merge_upstream(&repo).await {
                    Ok(Merge::Updated) => Outcome::Ok,
                    Ok(Merge::Unchanged) => Outcome::Skip,
                    Err(err) => Outcome::Failed(err.to_string()),
                };
                (repo.full_name, prefix, counter, bar, outcome)
            });
        }
        if running.is_empty() {
            break;
        }
        if let Some(result) = running.join_next().await {
            let (name, prefix, counter, bar, outcome) = result?;
            match outcome {
                Outcome::Ok => {
                    summary.ok += 1;
                    state.mark_checked(&name);
                    finish_line(
                        &bar,
                        format!("{prefix} {} Synced{counter}", status("DONE", "green", tty)),
                        tty,
                        json,
                    );
                }
                Outcome::Skip => {
                    summary.skipped += 1;
                    state.mark_checked(&name);
                    finish_line(
                        &bar,
                        format!(
                            "{prefix} {} Already up to date{counter}",
                            status("SKIP", "yellow", tty)
                        ),
                        tty,
                        json,
                    );
                }
                Outcome::Failed(error) => {
                    summary.failed += 1;
                    finish_line(
                        &bar,
                        format!("{prefix} {} {error}{counter}", status("FAILED", "red", tty)),
                        tty,
                        json,
                    );
                }
            }
            if let Some(overall) = &overall {
                overall.set_message(format!(
                    "{} synced · {} failed · {} skipped",
                    summary.ok, summary.failed, summary.skipped
                ));
                overall.inc(1);
            }
            // Checkpoint after each completed operation. Failures must remain eligible.
            state.save()?;
        }
    }
    for (index, repo) in queue {
        let prefix = repo_prefix(name_width, &repo.full_name);
        let counter = repo_counter(index, total);
        let bar = multiprogress.add(ProgressBar::new_spinner());
        bar.set_style(style.clone());
        finish_line(
            &bar,
            format!(
                "{prefix} {} Run limit reached{counter}",
                status("SKIP", "yellow", tty)
            ),
            tty,
            json,
        );
        summary.skipped += 1;
        if let Some(overall) = &overall {
            overall.inc(1);
        }
    }
    if !json {
        println!(
            "Summary · {} synced · {} failed · {} skipped",
            summary.ok, summary.failed, summary.skipped
        );
    }
    if let Some(overall) = overall {
        overall.finish_and_clear();
    }
    Ok(summary)
}

fn repo_prefix(name_width: usize, name: &str) -> String {
    format!("  {name:<name_width$}  ")
}

fn repo_counter(index: usize, total: usize) -> String {
    if index + 1 == total {
        format!(" · [{}/{total}]", index + 1)
    } else {
        String::new()
    }
}

fn status(label: &str, color: &str, tty: bool) -> String {
    if !tty {
        return format!("[{label}]");
    }
    let code = match color {
        "green" => 32,
        "yellow" => 33,
        "red" => 31,
        _ => 36,
    };
    format!("\x1b[{code}m[{label}]\x1b[0m")
}

fn finish_line(bar: &ProgressBar, message: String, tty: bool, json: bool) {
    if json {
        bar.finish_and_clear();
    } else if tty {
        bar.finish_with_message(message);
    } else {
        println!("{message}");
        bar.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skips_do_not_count() {
        let summary = Summary {
            ok: 2,
            failed: 1,
            skipped: 10,
        };
        assert_eq!(summary.counted(), 3);
    }
}
