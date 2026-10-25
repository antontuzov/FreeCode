//! Sampling and reasoning-effort defaults, straight from the Bonsai 2 27B model card.
//!
//! These are persisted defaults, overridable in Settings › Model settings. The engine launcher
//! (`freecode-llama`) always passes them explicitly because the GGUF carries only
//! `top_k`/`top_p`/`temperature` — `min_p`, `presence_penalty` and `repetition_penalty` are absent
//! from the file, so a server started without options would not apply them.

use serde::{Deserialize, Serialize};

/// Sampler set for one generation mode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingProfile {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub presence_penalty: f32,
    pub repetition_penalty: f32,
}

impl SamplingProfile {
    /// Thinking mode, model card + verified in Bonsai-demo `scripts/run_llama.sh` for the bonsai2
    /// family.
    pub const THINKING: SamplingProfile = SamplingProfile {
        temperature: 1.0,
        top_p: 0.95,
        top_k: 20,
        min_p: 0.05,
        presence_penalty: 0.0,
        repetition_penalty: 1.0,
    };

    /// Instruct / non-thinking mode, model card.
    pub const INSTRUCT: SamplingProfile = SamplingProfile {
        temperature: 0.7,
        top_p: 0.80,
        top_k: 20,
        min_p: 0.0,
        presence_penalty: 1.5,
        repetition_penalty: 1.0,
    };
}

/// Reasoning effort as the UI exposes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    /// Take the effort the selected model publishes as its own default. The alternative is a number
    /// saved while a different model was selected — which is what a shipped default becomes, the
    /// card behind these values describing the 27B while the app runs the 1.7B — and every launch
    /// then has to announce that it substituted something nobody chose.
    #[default]
    Auto,
    Low,
    Medium,
    Xhigh,
    /// Thinking disabled. Only honoured when the server runs `--reasoning auto`; `--reasoning on`
    /// overrides it client-side.
    None,
}

impl ReasoningEffort {
    /// The value an explicit choice puts on the chat template, or `None` for [`Self::Auto`], which
    /// has no value until a model has been resolved against its catalog entry.
    ///
    /// `low` is in the template's vocabulary but measured to consume about as many tokens as the
    /// default `xhigh` (KNOWN_ISSUES.md), so we send `xhigh` and label the control accordingly
    /// rather than promising a shorter trace. `high` is never sent: the template rejects it with
    /// HTTP 500.
    pub fn wire_value(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Low | Self::Xhigh => Some("xhigh"),
            Self::Medium => Some("medium"),
            Self::None => Some("none"),
        }
    }

    /// True when the UI must explain that the choice behaves like another one.
    pub fn is_downgraded(self) -> bool {
        matches!(self, Self::Low)
    }

    pub fn tooltip(self) -> Option<&'static str> {
        self.is_downgraded()
            .then_some("Low is unsupported by this model: it reasons about as long as Extra high.")
    }

    pub fn all() -> [Self; 5] {
        [Self::Auto, Self::Low, Self::Medium, Self::Xhigh, Self::None]
    }
}

/// Generation budget. A cap that is too small ends the run mid-thought and yields an empty reply —
/// the single most common report in KNOWN_ISSUES.md — so the floor is enforced, not suggested.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationBudget {
    pub max_tokens: u32,
    pub context_tokens: u32,
}

impl Default for GenerationBudget {
    fn default() -> Self {
        Self {
            // Model card "start here" table: `-n 16384` or more with `-c 65536`.
            max_tokens: 16_384,
            context_tokens: 32_768,
        }
    }
}

impl GenerationBudget {
    pub const MIN_MAX_TOKENS: u32 = 4_096;

    /// Clamp a user value up to the floor that keeps a thinking trace from being cut off.
    pub fn clamped(&self) -> Self {
        let max_tokens = self.max_tokens.max(Self::MIN_MAX_TOKENS);
        Self {
            max_tokens,
            context_tokens: self.context_tokens.max(max_tokens + 1_024),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_defaults_match_model_card() {
        let p = SamplingProfile::THINKING;
        assert_eq!(
            (
                p.temperature,
                p.top_p,
                p.top_k,
                p.min_p,
                p.presence_penalty,
                p.repetition_penalty
            ),
            (1.0, 0.95, 20, 0.05, 0.0, 1.0)
        );
    }

    #[test]
    fn instruct_defaults_match_model_card() {
        let p = SamplingProfile::INSTRUCT;
        assert_eq!(
            (
                p.temperature,
                p.top_p,
                p.top_k,
                p.min_p,
                p.presence_penalty,
                p.repetition_penalty
            ),
            (0.7, 0.80, 20, 0.0, 1.5, 1.0)
        );
    }

    #[test]
    fn low_maps_to_xhigh_and_high_is_never_sent() {
        assert_eq!(ReasoningEffort::Low.wire_value(), Some("xhigh"));
        assert!(ReasoningEffort::Low.tooltip().is_some());
        assert_eq!(ReasoningEffort::Xhigh.wire_value(), Some("xhigh"));
        assert_eq!(ReasoningEffort::Medium.wire_value(), Some("medium"));
        assert_eq!(ReasoningEffort::None.wire_value(), Some("none"));
        assert!(!ReasoningEffort::all()
            .iter()
            .any(|e| e.wire_value() == Some("high")));
    }

    /// The default has no wire value of its own, which is the whole point: it is the one setting
    /// that cannot go stale when the selected model changes.
    #[test]
    fn auto_carries_no_value_until_a_model_resolves_it() {
        assert_eq!(ReasoningEffort::default(), ReasoningEffort::Auto);
        assert_eq!(ReasoningEffort::Auto.wire_value(), None);
        assert_eq!(
            serde_json::to_string(&ReasoningEffort::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::from_str::<ReasoningEffort>("\"auto\"").unwrap(),
            ReasoningEffort::Auto
        );
    }

    #[test]
    fn budget_floor_prevents_truncated_thinking() {
        let tight = GenerationBudget {
            max_tokens: 256,
            context_tokens: 4_096,
        };
        let fixed = tight.clamped();
        assert_eq!(fixed.max_tokens, GenerationBudget::MIN_MAX_TOKENS);
        assert!(fixed.context_tokens > fixed.max_tokens);
    }
}
