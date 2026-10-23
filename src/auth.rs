use anyhow::{bail, Context, Result};
use inquire::Password;

const SERVICE: &str = "forksync.github.com";
const ACCOUNT: &str = "github-token";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).context("System credential store is unavailable")
}

pub fn token() -> Result<Option<String>> {
    for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Ok(Some(value));
            }
        }
    }
    match entry()?.get_password() {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => {
            bail!("Could not read secure credential store: {error}. You can also use GH_TOKEN.")
        }
    }
}

pub fn login() -> Result<String> {
    println!("GitHub sign-in · personal access token (PAT)");
    println!("Token needs access to forks and Contents: write permission.");
    let secret = Password::new("GitHub token (hidden):")
        .without_confirmation()
        .prompt()?;
    if secret.trim().is_empty() {
        bail!("Token cannot be empty");
    }
    Ok(secret)
}

pub fn store(token: &str) -> Result<()> {
    entry()?.set_password(token).context("Could not save token to secure system storage. Use the GH_TOKEN environment variable instead")
}

pub fn logout() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
