use anyhow::{bail, Context, Result};
use reqwest::{header, Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub full_name: String,
    pub default_branch: String,
    pub fork: bool,
    pub archived: bool,
    pub disabled: bool,
}

#[derive(Debug, Deserialize)]
struct User {
    login: String,
}

#[derive(Debug, Deserialize)]
struct MergeReply {
    message: Option<String>,
}

#[derive(Clone)]
pub struct Github {
    client: Client,
    base_url: String,
}

#[derive(Debug)]
pub enum Merge {
    Updated,
    Unchanged,
}

impl Github {
    pub fn new(token: &str) -> Result<Self> {
        let base_url = std::env::var("GITHUB_API_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://api.github.com".to_owned());
        Self::new_with_base(token, base_url)
    }

    pub fn new_with_base(token: &str, base_url: String) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::ACCEPT, "application/vnd.github+json".parse()?);
        headers.insert("X-GitHub-Api-Version", "2026-03-10".parse()?);
        headers.insert(header::USER_AGENT, "forksync".parse()?);
        let bearer = format!("Bearer {token}");
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&bearer).context("Érvénytelen token formátum")?,
        );
        // Never print headers or reqwest::Request when reporting failures.
        Ok(Self {
            client: Client::builder()
                .default_headers(headers)
                .timeout(Duration::from_secs(30))
                .build()?,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        const MAX_ATTEMPTS: u32 = 5;
        for attempt in 0..MAX_ATTEMPTS {
            let mut req = self.client.request(method.clone(), url);
            if let Some(ref json) = body {
                req = req.json(json);
            }
            let response = match req.send().await {
                Ok(response) => response,
                Err(error)
                    if attempt + 1 < MAX_ATTEMPTS
                        && (error.is_connect() || error.is_timeout() || error.is_request()) =>
                {
                    tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };

            if response.status().is_success() {
                return Ok(response);
            }

            let status = response.status();
            let retry_after = retry_after_seconds(response.headers());
            let transient = status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::REQUEST_TIMEOUT
                || status.is_server_error();
            if transient && attempt + 1 < MAX_ATTEMPTS {
                let delay = retry_after.unwrap_or_else(|| 2_u64.pow(attempt).min(30));
                tokio::time::sleep(Duration::from_secs(delay.min(120))).await;
                continue;
            }
            return Ok(response);
        }
        bail!("Sikertelen GitHub API-kérés: {url}")
    }

    pub async fn viewer(&self) -> Result<String> {
        let res = self
            .request(
                reqwest::Method::GET,
                &format!("{}/user", self.base_url),
                None,
            )
            .await?;
        if !res.status().is_success() {
            bail!("GitHub hitelesítési hiba: HTTP {}", res.status());
        }
        Ok(res.json::<User>().await?.login)
    }

    pub async fn forks(&self, owner: &str, viewer: &str) -> Result<Vec<Repo>> {
        let path = if viewer.eq_ignore_ascii_case(owner) {
            format!(
                "{}/user/repos?affiliation=owner&sort=full_name",
                self.base_url
            )
        } else {
            format!(
                "{}/orgs/{owner}/repos?type=all&sort=full_name",
                self.base_url
            )
        };
        // Organizations use /orgs; other users use /users (public repositories only).
        match self.pages(&path, owner).await {
            Ok(repos) => Ok(repos),
            Err(e) if !viewer.eq_ignore_ascii_case(owner) && e.to_string().contains("HTTP 404") => {
                self.pages(
                    &format!(
                        "{}/users/{owner}/repos?type=owner&sort=full_name",
                        self.base_url
                    ),
                    owner,
                )
                .await
            }
            Err(e) => Err(e),
        }
    }

    async fn pages(&self, base: &str, owner: &str) -> Result<Vec<Repo>> {
        let mut forks = Vec::new();
        for page in 1..=1000 {
            let url = format!("{base}&per_page=100&page={page}");
            let response = self.request(reqwest::Method::GET, &url, None).await?;
            if !response.status().is_success() {
                bail!("A forkok listázása sikertelen: HTTP {}", response.status());
            }
            let chunk: Vec<Repo> = response
                .json()
                .await
                .context("Hibás GitHub repository-lista")?;
            let count = chunk.len();
            forks.extend(chunk.into_iter().filter(|r| {
                r.fork
                    && r.full_name
                        .split('/')
                        .next()
                        .is_some_and(|n| n.eq_ignore_ascii_case(owner))
            }));
            if count < 100 {
                break;
            }
        }
        forks.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        Ok(forks)
    }

    pub async fn merge_upstream(&self, repo: &Repo) -> Result<Merge> {
        let url = format!("{}/repos/{}/merge-upstream", self.base_url, repo.full_name);
        let response = self
            .request(
                reqwest::Method::POST,
                &url,
                Some(serde_json::json!({"branch": repo.default_branch})),
            )
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let message = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("GitHub API hiba");
            bail!("HTTP {status}: {}", message.replace(['\r', '\n'], " "));
        }
        let message = response
            .json::<MergeReply>()
            .await?
            .message
            .unwrap_or_default();
        let normalized = message.to_ascii_lowercase();
        if normalized.contains("not behind")
            || normalized.contains("already up to date")
            || normalized.contains("up-to-date")
        {
            Ok(Merge::Unchanged)
        } else {
            Ok(Merge::Updated)
        }
    }
}

fn retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let text = value.to_str().ok()?;
    if let Ok(seconds) = text.trim().parse::<u64>() {
        return Some(seconds);
    }
    chrono::DateTime::parse_from_rfc2822(text.trim())
        .ok()
        .map(|date| {
            let now = chrono::Utc::now();
            (date.timestamp() - now.timestamp()).max(0) as u64
        })
        .filter(|seconds| *seconds > 0)
}
