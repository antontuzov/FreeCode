//! The free cloud models the picker offers, and the one gateway that serves them.
//!
//! The list is the free tier OpenCode publishes, read against their model page; a row is routable
//! through the gateway only when the same model has a verified OpenRouter id, because the relay sends
//! through that one gateway and a guessed id is a request to a model that is not there. The promo
//! models OpenCode serves only inside their own product therefore carry no upstream id: the picker
//! shows them as not routable through the key rather than pretending a route exists. They are still
//! answerable through the OpenCode app on this Mac, which is the runtime those models are reserved
//! for — see [`crate::opencode`].
//!
//! The gateway key follows the same rule as the remote provider's: it lives in the platform
//! keychain, the UI only ever learns whether one is saved, and it is loaded at request time for the
//! one request that uses it.

use serde::Serialize;

#[cfg(target_os = "macos")]
const SERVICE: &str = "studio.smollm.desktop.free-models";
#[cfg(target_os = "macos")]
const ACCOUNT: &str = "openrouter-key";

/// The one gateway this build routes free turns through. One key slot is the whole secret surface.
pub const GATEWAY_BASE: &str = "https://openrouter.ai/api/v1";

/// One free model, as the picker and the relay see it.
#[derive(Debug)]
pub struct FreeModel {
    pub id: &'static str,
    pub label: &'static str,
    pub provider: &'static str,
    /// The OpenRouter model id, present only when a page for it was verified there.
    pub upstream: Option<&'static str>,
    /// Fallback order: a failed turn moves to the next higher priority, never back.
    pub priority: u8,
}

/// What the frontend needs to draw a row. `is_free` is constant here — this is the free list — but
/// the field keeps the row's shape explicit rather than implied by the group it renders in.
///
/// `routable` is about the gateway key: a row whose model OpenRouter does not list has no upstream
/// id to send to. `local` is the other route, the OpenCode app on this Mac, which serves the whole
/// list — so a row can be unpickable for the key and pickable for the app in the same breath, and the
/// picker says which.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeModelInfo {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub is_free: bool,
    pub routable: bool,
    /// Whether the OpenCode app's local service is answering, which makes every row answerable.
    pub local: bool,
}

/// The free tier OpenCode publishes, in their page's order.
pub fn models() -> &'static [FreeModel] {
    &[
        FreeModel {
            id: "big-pickle",
            label: "Big Pickle",
            provider: "OpenCode",
            upstream: None,
            priority: 1,
        },
        FreeModel {
            id: "glm-5.3-flash-free",
            label: "GLM-5.3-Flash",
            provider: "GLM",
            upstream: Some("z-ai/glm-5.3-flash-free"),
            priority: 2,
        },
        FreeModel {
            id: "deepseek-v4.1-flash-free",
            label: "DeepSeek V4.1 Flash",
            provider: "DeepSeek",
            upstream: Some("deepseek/deepseek-v4.1-flash"),
            priority: 3,
        },
        FreeModel {
            id: "beam-free",
            label: "Beam",
            provider: "Beam",
            upstream: None,
            priority: 4,
        },
        FreeModel {
            id: "flint-40b-free",
            label: "Flint 40B",
            provider: "Flint",
            upstream: None,
            priority: 5,
        },
        FreeModel {
            id: "longcat-2.5-preview-free",
            label: "LongCat 2.5 Preview",
            provider: "LongCat",
            upstream: None,
            priority: 6,
        },
        FreeModel {
            id: "exo-free",
            label: "Exo",
            provider: "Exo",
            upstream: None,
            priority: 7,
        },
        FreeModel {
            id: "mimo-v2.6-flash-free",
            label: "MiMo-V2.6-Flash",
            provider: "MiMo",
            upstream: Some("xiaomi/mimo-v2.6-flash"),
            priority: 8,
        },
        FreeModel {
            id: "mimo-v2.5-free",
            label: "MiMo V2.5",
            provider: "MiMo",
            upstream: None,
            priority: 9,
        },
        FreeModel {
            id: "nemotron-3-ultra-free",
            label: "Nemotron 3 Ultra",
            provider: "NVIDIA",
            upstream: Some("nvidia/nemotron-3-ultra-550b-a55b:free"),
            priority: 10,
        },
        FreeModel {
            id: "nemotron-3.5-lightning-free",
            label: "Nemotron 3.5 Lightning",
            provider: "NVIDIA",
            upstream: Some("nvidia/nemotron-3.5-lightning:free"),
            priority: 11,
        },
        FreeModel {
            id: "nemotron-3.5-super-vl-free",
            label: "Nemotron 3.5 Super VL",
            provider: "NVIDIA",
            upstream: Some("nvidia/nemotron-3.5-super-vl"),
            priority: 12,
        },
        FreeModel {
            id: "jev-1.13-free",
            label: "JEV 1.13",
            provider: "TypeSafe AI",
            upstream: None,
            priority: 13,
        },
    ]
}

pub fn find(id: &str) -> Option<&'static FreeModel> {
    models().iter().find(|model| model.id == id)
}

pub fn infos(local: bool) -> Vec<FreeModelInfo> {
    models()
        .iter()
        .map(|model| FreeModelInfo {
            id: model.id.to_string(),
            label: model.label.to_string(),
            provider: model.provider.to_string(),
            is_free: true,
            routable: model.upstream.is_some(),
            local,
        })
        .collect()
}

/// The routable models the relay may try for a turn, starting at the chosen one and moving to the
/// next priority only after a failure. A model with no upstream id never appears: there is nothing
/// to send its request to, and the refusal says so before any bytes leave the machine. An id the
/// registry does not hold has no order at all rather than the whole routable list: only a pick the
/// host resolved is ever sent.
pub fn attempt_order(id: &str) -> Vec<&'static FreeModel> {
    let Some(start) = models()
        .iter()
        .find(|model| model.id == id)
        .map(|model| model.priority)
    else {
        return Vec::new();
    };
    models()
        .iter()
        .filter(|model| model.priority >= start && model.upstream.is_some())
        .collect()
}

#[cfg(target_os = "macos")]
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| format!("the macOS Keychain is not available: {e}"))
}

#[cfg(target_os = "macos")]
pub fn save_key(raw: &str) -> Result<(), String> {
    let key = raw.trim();
    if key.is_empty() {
        return Err("the key is empty".into());
    }
    if key.chars().any(char::is_control) {
        return Err(
            "the key contains a control character; paste it again without line breaks".into(),
        );
    }
    entry()?
        .set_password(key)
        .map_err(|e| format!("the macOS Keychain refused the key: {e}"))
}

#[cfg(target_os = "macos")]
pub fn load_key() -> Result<String, String> {
    match entry()?.get_password() {
        Ok(key) if !key.is_empty() => Ok(key),
        Ok(_) => Err("the saved gateway key is empty; paste it again".into()),
        Err(keyring::Error::NoEntry) => {
            Err("no OpenRouter key is saved; paste one under Settings → Model → Free models".into())
        }
        Err(e) => Err(format!("the macOS Keychain would not return the key: {e}")),
    }
}

#[cfg(target_os = "macos")]
pub fn key_saved() -> bool {
    matches!(
        entry().and_then(|e| e.get_password().map_err(|e| e.to_string())),
        Ok(key) if !key.is_empty()
    )
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
    Err("this build has no secure key store; a gateway key is macOS-only for now".into())
}

#[cfg(not(target_os = "macos"))]
pub fn load_key() -> Result<String, String> {
    Err("this build has no secure key store; a gateway key is macOS-only for now".into())
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
    fn the_registry_names_each_model_once() {
        let list = models();
        let ids: Vec<_> = list.iter().map(|model| model.id).collect();
        let labels: Vec<_> = list.iter().map(|model| model.label).collect();
        for id in &ids {
            assert_eq!(
                ids.iter().filter(|held| *held == id).count(),
                1,
                "{id} twice"
            );
        }
        for label in &labels {
            assert_eq!(
                labels.iter().filter(|held| *held == label).count(),
                1,
                "{label} twice"
            );
        }
    }

    #[test]
    fn a_routable_model_names_a_gateway_id_with_a_provider() {
        for model in models() {
            if let Some(upstream) = model.upstream {
                assert!(
                    upstream.contains('/'),
                    "{} upstream id lacks a provider prefix",
                    model.id
                );
            }
        }
    }

    #[test]
    fn the_attempt_order_moves_forward_only() {
        let order = attempt_order("deepseek-v4.1-flash-free");
        assert_eq!(order[0].id, "deepseek-v4.1-flash-free");
        assert!(order.iter().all(|model| model.upstream.is_some()));
        let priorities: Vec<_> = order.iter().map(|model| model.priority).collect();
        let mut sorted = priorities.clone();
        sorted.sort_unstable();
        assert_eq!(priorities, sorted);
        // An unknown id has no attempt order: nothing can be sent.
        assert!(attempt_order("not-a-model").is_empty());
    }

    #[test]
    fn the_promo_models_stay_listed_but_unroutable() {
        let big_pickle = find("big-pickle").unwrap();
        assert_eq!(big_pickle.upstream, None);
        assert!(infos(false)
            .iter()
            .find(|info| info.id == "big-pickle")
            .is_some_and(|info| !info.routable && info.is_free));
        // The OpenCode app on this Mac serves the promotion rows too, so the picker marks the whole
        // list as answerable when the service is up.
        assert!(infos(true).iter().all(|info| info.local));
    }
}
