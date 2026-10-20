//! The remote provider's credentials and address.
//!
//! A remote prompt is the one path where something the user typed leaves this machine, and the API
//! key is the one secret the app is handed. It therefore never enters `settings.json`, never crosses
//! back over the IPC boundary and never appears in a log line here: [`save_key`] writes it to the
//! platform keychain, [`load_key`] reads it for the request that needs it, and the UI only ever learns
//! whether one is present.
//!
//! The address is validated here as well. HTTPS is required because a key sent over plain HTTP is a
//! key given to everyone on the way; the two loopback spellings are the exception, so a local
//! OpenAI-compatible server on this machine remains reachable without a certificate.

use reqwest::Url;

/// Keychain service/account pair. One provider at a time is this build's contract; changing the base
/// URL edits the address the saved key is sent to, it does not create a second credential.
const SERVICE: &str = "studio.smollm.desktop.remote-provider";
const ACCOUNT: &str = "api-key";

/// The base URL as stored and as used, with surrounding space and the trailing slash removed.
///
/// Whitespace inside the URL is rejected rather than escaped: a pasted address with a newline in the
/// middle is a mistake the user can see, while silently repairing it would hide where the request
/// actually went.
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("a remote provider needs a base URL".into());
    }
    let parsed = Url::parse(trimmed).map_err(|_| {
        "the base URL is not a URL; include the scheme, e.g. https://api.openai.com/v1".to_string()
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("put credentials in the key field, not in the base URL".into());
    }
    if parsed.host_str().is_none() {
        return Err("the base URL has no host".into());
    }
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    match parsed.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => {
            return Err(
                "plain HTTP would send the key in the clear; use HTTPS, or a localhost address"
                    .into(),
            )
        }
        _ => return Err("the base URL must start with https://".into()),
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("the base URL must not carry a query or a fragment".into());
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// Refuse a value that parses as a URL carrying a user name or password.
///
/// The settings page saves on every keystroke, so an address is stored as typed rather than normalized
/// there — a half-typed URL is not an error to announce, and the send path applies the full rule before
/// a request exists. This one check does happen at save time: a key pasted into the address field
/// would otherwise be written to `settings.json` in clear text, which is the one thing the document
/// must never hold. A string that does not parse is left to the send-time refusal.
pub fn reject_embedded_credentials(raw: &str) -> Result<(), String> {
    if let Ok(parsed) = Url::parse(raw.trim()) {
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err("put credentials in the key field, not in the base URL".into());
        }
    }
    Ok(())
}

/// Whether a pasted key is something that can be stored and sent as a bearer token.
fn checked_key(raw: &str) -> Result<&str, String> {
    let key = raw.trim();
    if key.is_empty() {
        return Err("the key is empty".into());
    }
    if key.chars().any(char::is_control) {
        return Err(
            "the key contains a control character; paste it again without line breaks".into(),
        );
    }
    Ok(key)
}

#[cfg(target_os = "macos")]
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| format!("the macOS Keychain is not available: {e}"))
}

#[cfg(target_os = "macos")]
pub fn save_key(raw: &str) -> Result<(), String> {
    let key = checked_key(raw)?;
    entry()?
        .set_password(key)
        .map_err(|e| format!("the macOS Keychain refused the key: {e}"))
}

#[cfg(target_os = "macos")]
pub fn load_key() -> Result<String, String> {
    match entry()?.get_password() {
        Ok(key) if !key.is_empty() => Ok(key),
        Ok(_) => Err("the saved API key is empty; paste it again".into()),
        Err(keyring::Error::NoEntry) => {
            Err("no API key is saved; paste one under Settings → Model → Remote provider".into())
        }
        Err(e) => Err(format!("the macOS Keychain would not return the key: {e}")),
    }
}

#[cfg(target_os = "macos")]
pub fn key_saved() -> bool {
    matches!(entry().and_then(|e| e.get_password().map_err(|e| e.to_string())), Ok(key) if !key.is_empty())
}

#[cfg(target_os = "macos")]
pub fn clear_key() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("the macOS Keychain would not remove the key: {e}")),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn save_key(_raw: &str) -> Result<(), String> {
    Err("this build has no secure key store; a remote API key is macOS-only for now".into())
}

#[cfg(not(target_os = "macos"))]
pub fn load_key() -> Result<String, String> {
    Err("this build has no secure key store; a remote API key is macOS-only for now".into())
}

#[cfg(not(target_os = "macos"))]
pub fn key_saved() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn clear_key() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_address_must_be_https_or_loopback() {
        assert_eq!(
            normalize_base_url(" https://api.openai.com/v1/ ").unwrap(),
            "https://api.openai.com/v1"
        );
        assert!(normalize_base_url("http://localhost:8080/v1").is_ok());
        assert!(normalize_base_url("http://127.0.0.1:8080/v1").is_ok());
        assert!(normalize_base_url("http://api.example.com/v1").is_err());
        assert!(normalize_base_url("ftp://api.example.com").is_err());
        assert!(normalize_base_url("api.example.com/v1").is_err());
        assert!(normalize_base_url("").is_err());
    }

    #[test]
    fn credentials_or_a_query_belong_somewhere_else() {
        assert!(normalize_base_url("https://user:secret@api.example.com/v1").is_err());
        assert!(normalize_base_url("https://api.example.com/v1?key=secret").is_err());
        assert!(normalize_base_url("https://api.example.com/v1#frag").is_err());
    }

    #[test]
    fn a_saved_address_may_be_half_typed_but_never_carry_credentials() {
        assert!(reject_embedded_credentials("https://api.op").is_ok());
        assert!(reject_embedded_credentials("").is_ok());
        assert!(reject_embedded_credentials("https://user:secret@api.example.com/v1").is_err());
        assert!(
            reject_embedded_credentials("https://sk-live-in-the-host@api.example.com").is_err()
        );
    }

    #[test]
    fn a_key_is_trimmed_but_never_repaired() {
        assert_eq!(checked_key("  sk-test  ").unwrap(), "sk-test");
        assert!(checked_key("   ").is_err());
        assert!(checked_key("sk-test\nsk-other").is_err());
    }
}
