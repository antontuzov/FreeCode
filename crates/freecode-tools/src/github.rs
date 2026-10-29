//! The agent's route to GitHub: the credential it spends, and the pull request it opens with it.
//!
//! Two different things leave this machine through this door, and they leave it by two different
//! roads. The push is git's own: the executor runs `git push` with whatever credential this machine
//! already holds, so the app never handles a password it did not ask for. The pull request is an
//! API call, and the API wants a token — the user pastes one into the Keychain, the host reads it
//! for the one request that needs it, and the model never sees it.
//!
//! The token is reached through [`GitHubToken`] rather than an environment variable or an
//! argument, for the reason the remote provider's key is kept the way it is: a secret that crosses
//! the IPC boundary or lands in a process's argv is a secret that ends up in a log line.

use std::time::Duration;

use reqwest::Url;
use serde_json::{json, Value};

use crate::web::WebError;

/// The GitHub API's address. Fixed: a run may act on GitHub and nowhere else.
const API: &str = "https://api.github.com";

/// How long one API call may take before the round says it did not.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Where the pull-request token comes from.
pub trait GitHubToken: Send + Sync {
    /// The token to authenticate with, or `None` when the user has not saved one.
    fn token(&self) -> Option<String>;
}

/// One pull request, as the API describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub title: String,
}

/// The `owner/name` a remote points at, when that remote is GitHub.
///
/// Both spellings of a remote are understood — the HTTPS one and the SSH one — because which one a
/// repository uses is the owner's choice, not the app's. A remote on another host is `None`: the
/// route is GitHub's, and a guess about some other forge would be a request to the wrong API.
pub fn repo_from_remote(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let rest = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        rest
    } else {
        trimmed.strip_prefix("http://github.com/")?
    };
    let rest = rest.trim_end_matches('/');
    let name = rest.strip_suffix(".git").unwrap_or(rest);
    let name = name.trim_end_matches('/');
    if valid_repo(name) {
        Some(name.to_string())
    } else {
        None
    }
}

/// Whether a repository is shaped like `owner/name`.
///
/// The two segments are checked rather than trusted, because this string goes into a URL: a name
/// with a slash in it, or a dot-dot, would send the request somewhere else.
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    parts.next().is_none() && segment_ok(owner) && segment_ok(name)
}

/// Whether a branch is shaped like a branch name: no spaces, no leading dash, no `..`.
pub fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && !branch.starts_with('-')
        && !branch.contains("..")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

/// One path segment of a repository name: letters, digits, dots, dashes and underscores.
fn segment_ok(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The branch a repository's pull requests land on, asked of the API rather than guessed.
///
/// `main` and `master` are both wrong often enough to matter, and a pull request aimed at the wrong
/// base is a pull request nobody can merge.
pub fn default_branch(token: &str, repo: &str) -> Result<String, WebError> {
    let url = api_url(&format!("/repos/{repo}"))?;
    let body = send(
        token,
        reqwest::Method::GET,
        url,
        None,
        &format!("the repository {repo} could not be read"),
    )?;
    body.get("default_branch")
        .and_then(Value::as_str)
        .filter(|branch| valid_branch(branch))
        .map(str::to_string)
        .ok_or_else(|| {
            WebError::Failed(format!(
                "GitHub did not name a default branch for {repo}; name the base branch yourself"
            ))
        })
}

/// Open a pull request and answer with the one GitHub created.
pub fn open_pull_request(
    token: &str,
    repo: &str,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<PullRequest, WebError> {
    let url = api_url(&format!("/repos/{repo}/pulls"))?;
    let payload = json!({
        "title": title,
        "body": body,
        "head": head,
        "base": base,
        "draft": draft,
    });
    let answer = send(
        token,
        reqwest::Method::POST,
        url,
        Some(&payload),
        &format!("the pull request on {repo} could not be opened"),
    )?;
    let number = answer
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            WebError::Failed("GitHub answered without a pull-request number".to_string())
        })?;
    Ok(PullRequest {
        number,
        url: answer
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: answer
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(title)
            .to_string(),
    })
}

/// The API's address for one path.
fn api_url(path: &str) -> Result<Url, WebError> {
    Url::parse(&format!("{API}{path}"))
        .map_err(|e| WebError::Failed(format!("the GitHub address could not be built: {e}")))
}

/// One request, with the token, and the answer in words when the answer is a refusal.
fn send(
    token: &str,
    method: reqwest::Method,
    url: Url,
    payload: Option<&Value>,
    what: &str,
) -> Result<Value, WebError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("freecode/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| WebError::Failed(format!("the GitHub client could not start: {e}")))?;
    let mut request = client
        .request(method, url.clone())
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(payload) = payload {
        request = request.json(payload);
    }
    let response = request
        .send()
        .map_err(|e| WebError::Failed(format!("{what}: {e}")))?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(refusal(status, &text, what));
    }
    serde_json::from_str(&text)
        .map_err(|e| WebError::Failed(format!("GitHub answered in a shape not understood: {e}")))
}

/// What a refusal says, in the API's own words where it gave any.
///
/// The status alone is not an answer the model can act on: a 401 and a 403 are both "no", and what
/// to do about them is different. GitHub puts the reason in a `message` field, and it is quoted.
fn refusal(status: reqwest::StatusCode, text: &str, what: &str) -> WebError {
    let said = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let reason = match status.as_u16() {
        401 => "the saved GitHub token was refused; it may be expired, or revoked".to_string(),
        403 => "GitHub refused the request; the token may lack the `repo` scope, or the API rate \
                limit for this address is spent"
            .to_string(),
        404 => {
            "GitHub does not know this repository; the token may have no access to it".to_string()
        }
        422 => "GitHub would not accept the pull request".to_string(),
        _ => format!("GitHub answered {status}"),
    };
    if said.is_empty() {
        WebError::Failed(format!("{what}: {reason}"))
    } else {
        WebError::Failed(format!("{what}: {reason} — {said}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_remote_names_its_repository() {
        for url in [
            "https://github.com/antontuzov/FreeCode.git",
            "https://github.com/antontuzov/FreeCode",
            "git@github.com:antontuzov/FreeCode.git",
            "ssh://git@github.com/antontuzov/FreeCode.git",
            "  https://github.com/antontuzov/FreeCode/  ",
        ] {
            assert_eq!(
                repo_from_remote(url),
                Some("antontuzov/FreeCode".to_string()),
                "{url}"
            );
        }
    }

    #[test]
    fn a_remote_on_another_host_is_not_guessed_at() {
        for url in [
            "https://gitlab.com/owner/repo.git",
            "git@bitbucket.org:owner/repo.git",
            "/local/path/to/repo",
            "https://github.com/owner",
            "https://github.com/owner/repo/extra",
        ] {
            assert_eq!(repo_from_remote(url), None, "{url}");
        }
    }

    #[test]
    fn a_repository_name_is_two_plain_segments() {
        assert!(valid_repo("antontuzov/FreeCode"));
        assert!(valid_repo("owner/name.with.dots"));
        assert!(!valid_repo("owner"));
        assert!(!valid_repo("owner/name/extra"));
        assert!(!valid_repo("owner/"));
        assert!(!valid_repo("/name"));
        assert!(!valid_repo("owner/../name"));
        assert!(!valid_repo("owner/na me"));
    }

    #[test]
    fn a_branch_name_is_a_name_and_not_an_option() {
        assert!(valid_branch("main"));
        assert!(valid_branch("feature/fix-the-thing"));
        assert!(valid_branch("release-1.2"));
        assert!(!valid_branch(""));
        assert!(!valid_branch("--upload-pack=evil"));
        assert!(!valid_branch("has space"));
        assert!(!valid_branch("a..b"));
    }
}
