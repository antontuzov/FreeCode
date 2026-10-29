//! The free cloud models the picker offers, and the one service that answers them.
//!
//! The list is what `opencode models` prints under the `opencode/` provider, rechecked 2026-10-29
//! against the CLI the OpenCode app ships. These are that service's own free models: every row is
//! answered by the OpenCode app running on this Mac, which is the runtime they are reserved for, and
//! the relay talks to it over loopback — see [`crate::opencode`].
//!
//! There is deliberately no second route here. This build once carried an OpenRouter key and sent a
//! free turn there when the service was down; those ids were this app's guess at a mirror of
//! OpenCode's list rather than a route OpenCode publishes, and a row that named DeepSeek or NVIDIA
//! while asking a third party for it said something about those makers that was not true. So a free
//! model is pickable exactly when the service that serves it is answering, and a turn sent without it
//! is refused with the reason.

use serde::Serialize;

/// One free model, as the picker and the relay see it.
#[derive(Debug)]
pub struct FreeModel {
    pub id: &'static str,
    pub label: &'static str,
    pub provider: &'static str,
}

/// What the frontend needs to draw a row. `is_free` is constant here — this is the free list — but
/// the field keeps the row's shape explicit rather than implied by the group it renders in.
///
/// `local` is whether the OpenCode app's service is answering, which is the only way any of these
/// models can be asked. A row that is not `local` is listed and not pickable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeModelInfo {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub is_free: bool,
    /// Whether the OpenCode app's local service is answering, which is what makes a row pickable.
    pub local: bool,
}

/// The free tier OpenCode publishes, in their page's order.
pub fn models() -> &'static [FreeModel] {
    &[
        FreeModel {
            id: "big-pickle",
            label: "Big Pickle",
            provider: "FreeCode",
        },
        FreeModel {
            id: "glm-5.3-flash-free",
            label: "GLM-5.3-Flash",
            provider: "GLM",
        },
        FreeModel {
            id: "deepseek-v4.1-flash-free",
            label: "DeepSeek V4.1 Flash",
            provider: "DeepSeek",
        },
        FreeModel {
            id: "beam-free",
            label: "Beam",
            provider: "Beam",
        },
        FreeModel {
            id: "flint-40b-free",
            label: "Flint 40B",
            provider: "Flint",
        },
        FreeModel {
            id: "mimo-v2.6-flash-free",
            label: "MiMo-V2.6-Flash",
            provider: "MiMo",
        },
        FreeModel {
            id: "nemotron-3.5-lightning-free",
            label: "Nemotron 3.5 Lightning",
            provider: "NVIDIA",
        },
        FreeModel {
            id: "nemotron-3.5-super-vl-free",
            label: "Nemotron 3.5 Super VL",
            provider: "NVIDIA",
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
            local,
        })
        .collect()
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
        // What `opencode models` prints under the opencode/ provider, and nothing else: rows the
        // service has stopped listing (Exo, MiMo 2.5, JEV, Nemotron 3 Ultra, MiniMax M3.1, LongCat
        // 2.5) are rows that can only fail, so the list is checked against the running service
        // rather than kept from memory.
        assert_eq!(
            ids,
            [
                "big-pickle",
                "glm-5.3-flash-free",
                "deepseek-v4.1-flash-free",
                "beam-free",
                "flint-40b-free",
                "mimo-v2.6-flash-free",
                "nemotron-3.5-lightning-free",
                "nemotron-3.5-super-vl-free",
            ]
        );
    }

    #[test]
    fn every_row_names_the_maker_it_is_asked_of() {
        for model in models() {
            assert!(
                !model.provider.trim().is_empty(),
                "{} names no provider",
                model.id
            );
            // A row's id is what the service is switched to, so it has to be the bare model id
            // rather than a carrier-prefixed upstream id from some other gateway.
            assert!(
                !model.id.contains('/'),
                "{} carries a provider prefix, which is an upstream id, not a service one",
                model.id
            );
        }
    }

    #[test]
    fn the_settings_default_names_a_model_this_build_carries() {
        find(freecode_config::DEFAULT_FREE_MODEL_ID)
            .expect("the settings default must be a registry id");
    }

    #[test]
    fn the_service_is_the_only_thing_that_makes_a_row_pickable() {
        // With the OpenCode app not answering, no row can be asked — there is no gateway behind it.
        assert!(infos(false).iter().all(|info| !info.local));
        assert!(infos(true).iter().all(|info| info.local));
        // Every row is free, and none of them ever advertised a route of their own.
        assert!(infos(false).iter().all(|info| info.is_free));
    }
}
