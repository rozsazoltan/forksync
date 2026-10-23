"""Simple offline structural checks; not a substitute for cargo test."""
from pathlib import Path
r = Path(__file__).resolve().parents[1]
assert 'AGPL-3.0-or-later' in (r/'Cargo.toml').read_text()
assert 'version = "2026.10.0"' in (r/'Cargo.toml').read_text()
assert 'gh repo' not in (r/'src/github.rs').read_text()
assert 'graphql' not in (r/'src/github.rs').read_text().lower()
assert 'merge-upstream' in (r/'src/github.rs').read_text()
assert 'max_repos_per_run: 50' in (r/'src/config.rs').read_text()
assert 'grace_days: 7' in (r/'src/config.rs').read_text()
assert (r/'.github/workflows/release.yml').exists()
print('Source layout checks passed.')
