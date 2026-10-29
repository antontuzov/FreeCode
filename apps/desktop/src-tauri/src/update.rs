//! Whether a newer build of FreeCode has been published.
//!
//! The app is handed out as a file on a landing page, so a fix that ships as a new `.dmg` reaches
//! the people who already installed it only if they are told. This asks the release feed — one
//! request, made when the user presses the button, carrying nothing but the running version — and
//! answers with the published version next to this build's own.
//!
//! It is a question, not an installer. The bundle is ad-hoc signed rather than notarised, so a new
//! build is fetched and put in place by hand; the row says so instead of pretending to do it.

use serde::{Deserialize, Serialize};

/// The feed that names the latest release.
const LATEST_URL: &str = "https://api.github.com/repos/antontuzov/FreeCode/releases/latest";

/// Where a person goes to get the build. Used when the feed names no page of its own.
const RELEASES_URL: &str = "https://github.com/antontuzov/FreeCode/releases/latest";

/// How long the feed gets to answer before the row gives up.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What the feed said about the latest release.
#[derive(Debug, Deserialize)]
struct Release {
    /// The version the release was tagged with, such as `v0.1.3`.
    #[serde(default)]
    tag_name: String,
    /// The page a person opens to get the build.
    #[serde(default)]
    html_url: Option<String>,
    /// When it went out.
    #[serde(default)]
    published_at: Option<String>,
}

/// The published release next to this build's own version.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    /// The version of the running bundle.
    pub current: String,
    /// The version of the published release.
    pub latest: String,
    /// Whether the published release is newer than this build.
    pub available: bool,
    /// Where the newer build is downloaded from.
    pub url: String,
    /// When the release was published, when the feed says so.
    pub published_at: Option<String>,
}

/// Ask the release feed whether a newer build exists.
///
/// Nothing is polled: the request is made when the user asks for it, and it carries the running
/// version and nothing else — no key, no settings, no account.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Update, String> {
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .user_agent(format!("freecode/{current}"))
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| format!("the client could not be built: {e}"))?;
    let response = client
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("the request could not be made: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("the feed answered {status}"));
    }
    let body = response
        .text()
        .await
        .map_err(|e| format!("the answer could not be read: {e}"))?;
    let release = read_release(&body)?;
    let latest = release.tag_name.trim().trim_start_matches('v').to_string();
    Ok(Update {
        available: is_newer(&latest, &current),
        current,
        latest,
        url: release
            .html_url
            .filter(|page| !page.trim().is_empty())
            .unwrap_or_else(|| RELEASES_URL.to_string()),
        published_at: release.published_at,
    })
}

/// The published release, read out of the feed's answer.
///
/// The reasons are clauses rather than sentences: the row that shows one carries the sentence of its
/// own, and a reason that repeats that subject reads as two failures where there was one.
fn read_release(body: &str) -> Result<Release, String> {
    let release: Release =
        serde_json::from_str(body).map_err(|e| format!("the answer could not be read: {e}"))?;
    if release.tag_name.trim().is_empty() {
        return Err("the feed named no version".into());
    }
    Ok(release)
}

/// The numeric parts of a version: `v0.1.3`, `0.1.3` and `0.1.3-beta.1` all answer `(0, 1, 3)`.
///
/// A tag this cannot read is not a version, and a version this cannot read is not compared.
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let core = tag.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// Whether the published version is newer than the running build's.
///
/// An unreadable version answers "no": a tag this build cannot parse is no evidence that the build
/// is stale, and claiming a newer build on the strength of a string nobody understands is how an
/// update notice becomes a lie.
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_is_read_as_its_three_numbers() {
        assert_eq!(parse_version("v0.1.3"), Some((0, 1, 3)));
        assert_eq!(parse_version("0.1.3"), Some((0, 1, 3)));
        assert_eq!(parse_version(" 0.2 "), Some((0, 2, 0)));
        assert_eq!(parse_version("1"), Some((1, 0, 0)));
    }

    #[test]
    fn a_pre_release_is_read_as_the_version_it_leads_to() {
        assert_eq!(parse_version("v0.1.3-beta.1"), Some((0, 1, 3)));
        assert_eq!(parse_version("0.1.3+build7"), Some((0, 1, 3)));
    }

    #[test]
    fn a_tag_that_is_not_a_version_reads_as_nothing() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version("v0.x.1"), None);
    }

    #[test]
    fn a_newer_release_is_the_one_that_wins() {
        assert!(is_newer("0.1.3", "0.1.2"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
    }

    #[test]
    fn the_same_or_an_older_release_is_not_an_update() {
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
    }

    #[test]
    fn a_version_nobody_can_read_is_not_evidence_of_a_newer_build() {
        assert!(!is_newer("nightly", "0.1.2"));
        assert!(!is_newer("0.1.3", "nightly"));
    }

    #[test]
    fn the_feed_is_read_into_a_release() {
        let release = read_release(
            r#"{"tag_name":"v0.1.3","html_url":"https://github.com/antontuzov/FreeCode/releases/tag/v0.1.3","published_at":"2026-10-20T10:00:00Z"}"#,
        )
        .expect("the feed's answer is read");
        assert_eq!(release.tag_name, "v0.1.3");
        assert_eq!(
            release.published_at.as_deref(),
            Some("2026-10-20T10:00:00Z")
        );
    }

    #[test]
    fn a_feed_that_named_no_version_is_refused() {
        let reason = read_release(r#"{"html_url":"https://example.invalid"}"#)
            .expect_err("a release with no tag is not a release");
        assert!(
            reason.contains("no version"),
            "the reason names the gap: {reason}"
        );
    }

    #[test]
    fn an_answer_that_is_not_json_is_refused() {
        let reason = read_release("<html>not json</html>").expect_err("a page is not a release");
        assert!(
            reason.contains("could not be read"),
            "the reason says what went wrong: {reason}"
        );
    }
}
