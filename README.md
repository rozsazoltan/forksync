# forksync

Forksync keeps your GitHub forks in sync with their upstream repositories. It runs as a standalone executable and needs no GitHub CLI, Python, or PowerShell installation.

## Install

Download the archive for your operating system and architecture from the [latest release](https://github.com/rozsazoltan/forksync/releases/latest). Releases include Windows x64, Linux x64 and ARM64, and macOS Intel and Apple Silicon builds.

Extract the executable, place it in a directory on your `PATH`, and run `forksync`. On Windows, use `forksync.exe`. macOS may ask you to approve an unsigned download in Security settings.

## First run

```sh
forksync
```

Follow the prompts to set your GitHub username or organization and sign in. Forksync verifies your personal access token (PAT) and can save it to your operating system's credential store. On Linux, saving credentials requires a Secret Service provider such as GNOME Keyring or KWallet.

For a fine-grained PAT, grant access to the repositories you want to sync, with **Contents: Read and write** and **Metadata: Read-only** permissions. Organization repositories may also require approval or SSO authorization.

For automation, set `GH_TOKEN` or `GITHUB_TOKEN` through your shell or secret manager. `GH_TOKEN` takes precedence. Do not put tokens in `config.toml`; Forksync does not print them.

```sh
forksync auth login
forksync auth status
```

## Commands

```sh
forksync                         # Open the interactive menu
forksync sync                    # Sync eligible repositories now
forksync sync --owner OWNER      # Use another owner for this run
forksync failed list             # Show recorded failures and reasons
forksync failed check            # Retry only recorded failures
forksync config list             # Show settings
forksync config set owner OWNER  # Set your GitHub username or organization
forksync config path             # Show the configuration file path
forksync auth status             # Check sign-in status
forksync auth logout             # Remove the saved credential
forksync schedule status         # Check the daily schedule
```

`forksync failed check` retries GitHub sync operations and can change the listed forks. Forksync uses GitHub's upstream-merge API without force; it does not overwrite fork-specific commits. Conflicts and permission errors are recorded as failures. Use `forksync failed list` to review them before retrying.

For scripts and machine-readable output, use the global options before the command:

```sh
forksync --non-interactive sync  # Never prompt
forksync --no-progress sync      # Print finished repository rows only
forksync --json sync             # Print a JSON sync summary
forksync --json config list      # Print settings as JSON
```

## Settings

Use `forksync config set KEY VALUE` to change a setting. Defaults:

| Setting | Default | Purpose |
| --- | ---: | --- |
| `owner` | empty | GitHub username or organization |
| `max_repos_per_run` | `50` | Maximum successful or failed sync results per run |
| `concurrency` | `4` | Maximum concurrent sync tasks; range `1`–`32` |
| `min_wait_seconds` | `10` | Minimum delay between GitHub merge requests |
| `max_wait_seconds` | `15` | Maximum delay between GitHub merge requests |
| `grace_days` | `7` | Days before a successfully checked repository is eligible again |
| `schedule_time` | `09:00` | Daily schedule time in local time (`HH:MM`) |

Repositories skipped because they are archived, already current, or still within the grace period do not consume the run limit. Failed syncs are not cached by the grace period.

## Daily schedule

Set an owner and local run time, then install the operating system's scheduler:

```sh
forksync config set owner OWNER
forksync config set schedule_time 09:00
forksync schedule install
```

Check or remove the schedule with `forksync schedule status` and `forksync schedule remove`. Forksync uses Windows Task Scheduler, a Linux systemd user timer, or a macOS launchd user agent. Scheduled runs do not prompt; save a PAT in the operating system's credential store before installing the schedule. If you use an environment token instead, ensure the scheduler receives it. Reinstall the schedule if you move the executable.

## License & acknowledgements

Forksync is licensed under the [GNU Affero General Public License v3.0 or later](LICENSE). Thanks to the maintainers of the open-source Rust crates Forksync uses.
