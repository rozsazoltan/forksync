use crate::{
    config::Config,
    github::{Github, Merge, Repo},
    state::State,
};
use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use rand::Rng;
use std::{
    collections::VecDeque,
    io::{IsTerminal, Write},
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
    let _cursor = CursorGuard::new(tty);
    let style = ProgressStyle::with_template("{msg}{spinner:.cyan}")?;
    let finished_style = ProgressStyle::with_template("{msg}")?;
    let overall = if tty && total > 0 {
        let bar = ProgressBar::with_draw_target(Some(total as u64), ProgressDrawTarget::hidden());
        bar.set_style(ProgressStyle::with_template(
            "{spinner:.cyan} {wide_bar:.cyan/blue} {pos}/{len} repositories · {msg}",
        )?);
        bar.set_message("Preparing sync");
        Some(bar)
    } else {
        None
    };
    let mut overall_attached = false;
    let spacer_style = ProgressStyle::with_template(" ")?;
    let mut spacer = None;
    let pacer = Arc::new(Pacer::default());
    let mut queue: VecDeque<(usize, Repo)> = repos.into_iter().enumerate().collect();
    let mut running = JoinSet::new();
    let mut summary = Summary::default();

    if !json {
        println!(
            "forksync · syncing up to {} repositories · {} workers · {}-day cooldown",
            cfg.max_repos_per_run, cfg.concurrency, cfg.grace_days
        );
        println!();
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
            let bar = add_repo_bar(
                &multiprogress,
                &overall,
                &mut overall_attached,
                &mut spacer,
                &spacer_style,
                &style,
            );
            if tty {
                bar.enable_steady_tick(Duration::from_millis(100));
            }
            bar.set_message(format!("{prefix} "));
            if repo.archived || repo.disabled || repo.default_branch.is_empty() {
                finish_line(
                    &bar,
                    format!(
                        "{prefix} {} Archived, disabled, or empty repository{counter}",
                        status("SKIP", "yellow", tty)
                    ),
                    tty,
                    json,
                    &finished_style,
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
                    &finished_style,
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
                        &finished_style,
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
                        &finished_style,
                    );
                }
                Outcome::Failed(error) => {
                    summary.failed += 1;
                    state.clear_checked(&name);
                    finish_line(
                        &bar,
                        format!("{prefix} {} {error}{counter}", status("FAILED", "red", tty)),
                        tty,
                        json,
                        &finished_style,
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
        let bar = add_repo_bar(
            &multiprogress,
            &overall,
            &mut overall_attached,
            &mut spacer,
            &spacer_style,
            &style,
        );
        finish_line(
            &bar,
            format!(
                "{prefix} {} Run limit reached{counter}",
                status("SKIP", "yellow", tty)
            ),
            tty,
            json,
            &finished_style,
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

fn add_repo_bar(
    multiprogress: &MultiProgress,
    overall: &Option<ProgressBar>,
    overall_attached: &mut bool,
    spacer: &mut Option<ProgressBar>,
    spacer_style: &ProgressStyle,
    style: &ProgressStyle,
) -> ProgressBar {
    let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::hidden());
    bar.set_style(style.clone());
    if let Some(overall) = overall {
        if !*overall_attached {
            multiprogress.add(bar.clone());
            let blank = ProgressBar::with_draw_target(Some(1), ProgressDrawTarget::hidden());
            blank.set_style(spacer_style.clone());
            let blank = multiprogress.insert_after(&bar, blank);
            multiprogress.insert_after(&blank, overall.clone());
            overall.enable_steady_tick(Duration::from_millis(100));
            *spacer = Some(blank);
            *overall_attached = true;
        } else if let Some(spacer) = spacer {
            multiprogress.insert_before(spacer, bar.clone());
        } else {
            multiprogress.add(bar.clone());
        }
    } else {
        multiprogress.add(bar.clone());
    }
    bar
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

fn finish_line(
    bar: &ProgressBar,
    message: String,
    tty: bool,
    json: bool,
    finished_style: &ProgressStyle,
) {
    if json {
        bar.finish_and_clear();
    } else if tty {
        bar.set_style(finished_style.clone());
        bar.finish_with_message(message);
    } else {
        println!("{message}");
        bar.finish_and_clear();
    }
}

struct CursorGuard {
    enabled: bool,
}

impl CursorGuard {
    fn new(enabled: bool) -> Self {
        if enabled {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(b"\x1b[?25l");
            let _ = stdout.flush();
        }
        Self { enabled }
    }
}

impl Drop for CursorGuard {
    fn drop(&mut self) {
        if self.enabled {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(b"\x1b[?25h");
            let _ = stdout.flush();
        }
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
