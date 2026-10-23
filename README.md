# forksync

A lightweight, standalone Rust application for keeping GitHub forks synchronized with their upstream branches. Interactive menus are inspired by [wslim](https://github.com/datarose-dev/wslim). No `gh` CLI, Python, or PowerShell installation is required.

**Release/version:** `2026.10.0` (CalVer). **License:** `AGPL-3.0-or-later`.

## Platforms

GitHub Actions builds standalone executable files for Windows x64, Linux x64/ARM64, and macOS Intel/Apple Silicon. A native OS credential store is used when you choose to save a PAT; on Linux this requires a working Secret Service implementation (GNOME Keyring, KWallet, etc.). If a credential store is unavailable, provide `GH_TOKEN` or `GITHUB_TOKEN` instead. There is **no token or GitHub owner compiled into the executable**.

Copy the downloaded `forksync-<target>` executable to a directory on `PATH`, rename it to `forksync` (or `forksync.exe` on Windows), and run `forksync`. Common locations include `~/.local/bin` on Linux/macOS or a personal `bin` folder on Windows. No system-wide installation is needed. macOS unsigned builds may require manual approval in the operating system's security settings.

## First run

```sh
forksync
```

Use the interactive menu to set your GitHub username or organization, and sign in with a **GitHub personal access token (PAT)**. The token is entered in a hidden terminal prompt, verified against `/user` and stored in the operating system's credential store. No OAuth application or hardcoded client ID is needed. A fine-grained PAT must have **Contents: Read and write** access to the relevant fork repositories; additional metadata/read access is used to enumerate repositories. Organization access may require approval or SSO authorization.

For CI or headless environments:

```sh
# Provide GH_TOKEN securely via your shell/secret manager, not in config.toml.
forksync config set owner your-github-user-or-org
forksync sync
```

`GH_TOKEN` takes precedence over `GITHUB_TOKEN` and the credential store. The CLI never prints a token.

## Commands

```sh
forksync                            # Interactive menu
forksync sync                       # Run now (prompts only if needed on a TTY)
forksync sync --owner another-user  # One-run owner override
forksync sync --scheduled           # Noninteractive, once per local calendar day
forksync auth login                 # Enter + verify + securely save PAT
forksync auth status
forksync auth logout                # Delete only the saved OS credential
forksync config                     # Interactive settings
forksync config list
forksync config get owner
forksync config unset owner             # Restore the default, empty username
forksync --non-interactive sync         # Never prompt (for automation)
forksync --no-progress sync             # One final status line per repository
forksync --json sync                    # One JSON summary, no terminal progress
forksync --json config list             # Machine-readable settings
forksync config set max_repos_per_run 50
forksync config set concurrency 4
forksync config set min_wait_seconds 10
forksync config set max_wait_seconds 15
forksync config set grace_days 7
forksync config set schedule_time 09:00
forksync schedule install           # OS-native once-daily job
forksync schedule status
forksync schedule remove
```

### Configuration

`forksync config path` prints the platform-specific `config.toml` location. Defaults:

```toml
owner = ""
max_repos_per_run = 50
concurrency = 4
min_wait_seconds = 10
max_wait_seconds = 15
grace_days = 7
schedule_time = "09:00"
```

- `max_repos_per_run` is the number of **non-skipped results** (OK + FAILED), not the number discovered. Repositories skipped due to the grace period, archived state, already-current upstream, or missing branch **do not consume the limit**. Processing stops at the limit or when there are no repositories left.
- `concurrency` controls maximum in-flight sync tasks (1–32). A **global randomized pacing window** (10–15 seconds by default) separates outbound merge requests, even with concurrency > 1.
- `grace_days` tracks successful verification **per repository**, including 'already up to date'. Failed syncs are not cached. Each repository can be checked again once its grace period expires; this is independent of the daily run schedule.
- `schedule_time` is local wall-clock time. Windows uses Task Scheduler (`schtasks`); Linux uses a systemd **user** timer; macOS uses a launchd **user** agent. These are operating-system facilities, not additional background processes packaged with forksync.
- `--scheduled` skips a duplicate scheduled run on the same local day. Manual `forksync sync` runs are still allowed. A process lock prevents overlapping runs.

**Safety:** Syncing calls GitHub's `merge-upstream` REST endpoint without a force option. Divergent forks, conflicts, permission failures and other API errors are marked FAILED; the program does not overwrite fork-specific commits. Transient server errors are retried a limited number of times. No GraphQL dependency.

**GitHub API limitations:** GitHub's merge endpoint only handles a fork's tracked upstream branch. Archived, disabled and empty forks are skipped. API rates and permissions still apply. The application enumerates repositories from the authenticated owner's `/user/repos` endpoint or organization `/orgs/.../repos`; other users' repositories are limited to public `/users/.../repos` results.

## Terminal output

```text
forksync · 99 fork · max 50 művelet · 4 párhuzamos szál · 7 napos türelmi idő
 [ 1/99] owner/alpha [OK] Szinkronizálva
 [ 2/99] owner/bravo [SKIP] Türelmi idő
 [88/99] owner/charlie [FAILED] HTTP 409: Merge conflict
 [99/99] owner/delta [SKIP] Futási limit elérve
Összesítés: 49 OK · 1 FAILED · 49 SKIP (99 repo)
```

Every repository gets exactly one persistent progress line, starting as `[n/N] owner/repo ...` and updated in place to `[OK]`, `[FAILED]`, or `[SKIP]`. The left number is right-aligned to the width of the total count.

## OS scheduling notes

- **Linux:** `systemctl --user` requires a user manager. To run while logged out, enable lingering (`loginctl enable-linger <user>`), depending on distro policy.
- **macOS:** the launchd job runs when the user is logged into their GUI session. It does not wake a powered-off Mac.
- **Windows:** Task Scheduler uses the local user's context; session, security policy, sleep/wake, and stored credentials can affect unattended execution.
- Token prompts are **never** shown during scheduled runs. Store the PAT in the user's OS credential store first, or arrange for `GH_TOKEN` to be available to the scheduler (environment variables in an interactive shell may *not* carry into OS scheduler jobs).
- If the executable is moved, rerun `forksync schedule install` so the scheduled path is updated.

## Development

```sh
cargo fmt --all
cargo test
cargo clippy --all-targets -- -D warnings
cargo run
```

### CalVer releases (same policy as wslim)

Release versions use `YYYY.M.N`, with an unpadded month and the last component beginning at `0` each UTC month. For example, `v2026.10.0`, `v2026.10.1`, `v2026.11.0`. The next revision is computed from existing Git tags (the highest revision wins, not the number of releases).

Manual publishing: open **Actions → Release → Run workflow** from the default branch. The workflow updates `Cargo.toml` and `Cargo.lock`, commits `chore: release vYYYY.M.N`, adds an annotated tag, builds each supported OS/architecture, then publishes ZIP assets and `SHA256SUMS`. Pushing a manually prepared `vYYYY.M.N` tag is also supported, provided the tag exactly matches the package and lockfile versions. The release workflow verifies these values before publishing. Concurrent release runs are serialized, and a changed default branch causes an abort rather than overwriting new work.

The Python release helper is standard-library-only:

```sh
python3 scripts/calver.py next
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Published asset names are `forksync-windows-x86_64.zip`, `forksync-linux-x86_64.zip`, `forksync-linux-aarch64.zip`, `forksync-macos-x86_64.zip` and `forksync-macos-aarch64.zip`. Each archive contains one executable plus the full AGPL-3.0 license. These are suitable for discovery by [`marcosnils/bin`](https://github.com/marcosnils/bin):

```sh
bin install github.com/OWNER/forksync
```

Replace OWNER with the actual GitHub user or organization hosting the project. Before the first release, generate and commit `Cargo.lock` using `cargo generate-lockfile`, or let the manual release workflow generate it. **Release binaries are not included in this source archive**. Rust compilation and GitHub integration have not been exercised in the authoring environment.

## License

GNU Affero General Public License, version 3 or any later version (`AGPL-3.0-or-later`). The GitHub release workflow packages the [complete unmodified AGPL-3.0 license text](https://www.gnu.org/licenses/agpl-3.0.txt). The source snapshot currently contains a license notice; add the full local license text before source publication.
