//! `ModelStatus`: one round trip that answers "which route will the next prompt take, and can it
//! answer right now?".
//!
//! This build installs no model: every answer comes from the free route served by the OpenCode app
//! on this Mac, unless the user has pointed a turn at their own remote provider. So the card reports
//! the two routes a turn can resolve to, and the command adds the live state of the service behind
//! the free one — nothing here reads a catalog, a weights directory or a hardware probe, because
//! nothing is installed.

use serde::Serialize;

use freecode_config::{ModelSource, Settings};

use crate::free_models;

/// The provider the card shows when Remote is the selected source.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteView {
    pub base_url: String,
    pub model: String,
    pub context_tokens: u32,
    /// Whether the user's consent switch is on. Off means a remote turn is refused, not rerouted.
    pub allowed: bool,
    /// Whether the address and model are both present. A turn also needs consent and a saved key.
    pub configured: bool,
}

/// The free model the card shows when the picker's free tier is the selected source.
///
/// No secret is here, and no list of providers is: the registry is the host's, and a row is answered
/// by the local service or not at all — which the page reads from [`OpenCodeView`], not from here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeView {
    pub id: String,
    pub label: String,
    pub provider: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeView {
    /// Whether the OpenCode app's local service answers on this machine.
    pub available: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusView {
    /// The provider this turn goes to when Remote is the selected source. No secret is here: the
    /// Settings page asks the Keychain separately whether a key is saved, and this poll never reads
    /// one.
    pub remote: Option<RemoteView>,
    /// The free model this turn goes to. A document with no pick still names the built-in default,
    /// because that is the model the next prompt will reach.
    pub free: Option<FreeView>,
    /// The OpenCode app on this Mac, when it is running. It is the only thing that can answer a free
    /// pick, so the card has to be able to say whether the route the user is about to take exists.
    /// Filled by the command after this build, because it is a live process rather than a document.
    #[serde(default)]
    pub opencode: OpenCodeView,
}

pub fn build(settings: &Settings) -> ModelStatusView {
    let remote_selected = settings.remote_provider.source == ModelSource::Remote;
    let remote = remote_selected.then(|| RemoteView {
        base_url: settings.remote_provider.base_url.clone(),
        model: settings.remote_provider.model.clone(),
        context_tokens: settings.remote_provider.context_tokens,
        allowed: settings.privacy.allow_remote_providers,
        configured: settings.remote_provider.configured(),
    });
    let free = settings
        .free_model
        .as_deref()
        // A selected remote provider is the one source that stands in front of the free tier.
        .or_else(|| (!remote_selected).then_some(freecode_config::DEFAULT_FREE_MODEL_ID))
        .and_then(free_models::find)
        .map(|model| FreeView {
            id: model.id.to_string(),
            label: model.label.to_string(),
            provider: model.provider.to_string(),
        });

    ModelStatusView {
        remote,
        free,
        opencode: OpenCodeView::default(),
    }
}
