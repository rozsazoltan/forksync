use anyhow::{bail, Context, Result};
use inquire::Password;

const SERVICE: &str = "forksync.github.com";
const ACCOUNT: &str = "github-token";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .context("A rendszer hitelesítőadat-tárolója nem érhető el")
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
        Err(error) => bail!("Nem sikerült kiolvasni a biztonságos tokentárolót: {error}. Használható a GH_TOKEN változó is."),
    }
}

pub fn login() -> Result<String> {
    println!("GitHub bejelentkezés: személyes hozzáférési token (PAT).");
    println!("A tokennek hozzáférést kell adni a forkokhoz és a Contents: write jogosultsághoz.");
    let secret = Password::new("GitHub token (rejtett):")
        .without_confirmation()
        .prompt()?;
    if secret.trim().is_empty() {
        bail!("Üres token");
    }
    Ok(secret)
}

pub fn store(token: &str) -> Result<()> {
    entry()?.set_password(token).context("A token nem menthető a rendszer biztonságos tárolójába. Használd a GH_TOKEN környezeti változót")
}

pub fn logout() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
