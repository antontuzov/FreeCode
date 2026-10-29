//! The GitHub token, and where it lives.
//!
//! The agent's route to GitHub spends one secret, and it is spent exactly once: the pull request
//! call. The push needs none — git authenticates with whatever credential this machine already
//! holds for the remote — so this keychain holds a token for the API and nothing else.
//!
//! The rules are the remote provider's rules, because they are the same secret in the same place:
//! the token never enters `settings.json`, never crosses back over the IPC boundary and never
//! reaches the model. The page can ask whether one is saved; the host reads it for the one request
//! that needs it and hands it to the executor as a trait object rather than an argument.

use freecode_tools::GitHubToken;

/// Keychain service/account pair. One token, one account, one repository scope.
const SERVICE: &str = "studio.freecode.desktop.github";
const ACCOUNT: &str = "token";

/// The host's own answer to "where does the token come from".
///
/// The tool crate never reaches for the keychain itself — it asks this trait — so the credential
/// stays on the host's side of the line and the crate stays testable without one.
pub struct Token;

impl GitHubToken for Token {
    fn token(&self) -> Option<String> {
        load_token().ok()
    }
}

/// Whether a pasted token is something that can be stored and sent as a bearer token.
///
/// A GitHub token is `ghp_…` or `github_pat_…`, but this build does not insist on a prefix: a
/// fine-grained token and a classic one differ, and a token from a GitHub Enterprise server does
/// too. What is refused is the shape that is certainly a mistake — an empty paste, or one carrying
/// a line break, which is what a copy out of the browser's field usually is.
fn checked_token(raw: &str) -> Result<&str, String> {
    let token = raw.trim();
    if token.is_empty() {
        return Err("the token is empty".into());
    }
    if token.chars().any(char::is_control) {
        return Err(
            "the token contains a control character; paste it again without line breaks".into(),
        );
    }
    if token.chars().any(char::is_whitespace) {
        return Err("the token contains a space; paste it again without spaces".into());
    }
    Ok(token)
}

#[cfg(target_os = "macos")]
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| format!("the macOS Keychain is not available: {e}"))
}

#[cfg(target_os = "macos")]
pub fn save_token(raw: &str) -> Result<(), String> {
    let token = checked_token(raw)?;
    entry()?
        .set_password(token)
        .map_err(|e| format!("the macOS Keychain refused the token: {e}"))
}

#[cfg(target_os = "macos")]
pub fn load_token() -> Result<String, String> {
    match entry()?.get_password() {
        Ok(token) if !token.is_empty() => Ok(token),
        Ok(_) => Err("the saved GitHub token is empty; paste it again".into()),
        Err(keyring::Error::NoEntry) => {
            Err("no GitHub token is saved; paste one under Settings → Agent → GitHub token".into())
        }
        Err(e) => Err(format!(
            "the macOS Keychain would not return the token: {e}"
        )),
    }
}

#[cfg(target_os = "macos")]
pub fn token_saved() -> bool {
    matches!(entry().and_then(|e| e.get_password().map_err(|e| e.to_string())), Ok(token) if !token.is_empty())
}

#[cfg(target_os = "macos")]
pub fn clear_token() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!(
            "the macOS Keychain would not remove the token: {e}"
        )),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn save_token(_raw: &str) -> Result<(), String> {
    Err("this build has no secure key store; a GitHub token is macOS-only for now".into())
}

#[cfg(not(target_os = "macos"))]
pub fn load_token() -> Result<String, String> {
    Err("this build has no secure key store; a GitHub token is macOS-only for now".into())
}

#[cfg(not(target_os = "macos"))]
pub fn token_saved() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn clear_token() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_trimmed_but_never_repaired() {
        assert_eq!(checked_token("  ghp_test  ").unwrap(), "ghp_test");
        assert!(checked_token("   ").is_err());
        assert!(checked_token("ghp_test\nghp_other").is_err());
        assert!(checked_token("ghp test").is_err());
    }
}
