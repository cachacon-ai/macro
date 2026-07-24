//! Some predefined models backend convenience
//! Models are _not_ strictly verified. This is intentional so that we
//! can route to any <provider>/<model-id>
//!
//! FORK NOTE (BYOK): the semantic tiers are remapped to the fork's BYOK
//! providers — `Smart` is Kimi K3 and `Fast` is Kimi K2.7 Code, both on the
//! Kimi Code subscription endpoint's Anthropic-compatible surface (model ids
//! `k3` / `kimi-for-coding`; if you point `KIMI_BASE_URL` at the Kimi
//! Platform instead, the ids become `kimi-k3` / `kimi-k2.7-code` and the
//! strings below must change to match). The `Sonnet4_6` mid tier (memory
//! judge, call summarizer) is MiniMax M2.7 so judge workloads run on a
//! different provider than the generation model. The Anthropic/OpenAI
//! variants below still route normally if those keys are configured.

use crate::model::types::Model;
use rig_core::providers::anthropic::completion::CLAUDE_OPUS_4_7;
use rig_core::providers::openai::{GPT_5_5, GPT_5_MINI};
use serde::Serialize;
use utoipa::ToSchema;

static ANTHROPIC: &str = "anthropic";
static OPENAI: &str = "openai";
static KIMI: &str = "kimi";
static MINIMAX: &str = "minimax";
const CLAUDE_SONNET_5: &str = "claude-sonnet-5";
/// FORK: Smart tier — Kimi K3 on the Kimi Code endpoint.
const KIMI_K3: &str = "k3";
/// FORK: Fast tier — Kimi K2.7 Code on the Kimi Code endpoint.
const KIMI_K27_CODE: &str = "kimi-for-coding";
/// FORK: mid tier for judges/summaries — MiniMax M2.7 (~200K context).
const MINIMAX_M27: &str = "MiniMax-M2.7";

/// This type is **serialize-only**: every variant's wire form is the
/// provider's **api id** — the exact string the API (and the model router)
/// expects. The two semantic tiers (`Smart` / `Fast`) are server-side
/// concepts that resolve to a concrete model, so they serialize to that
/// model's api id too — the router never sees a semantic name, only an id it
/// can dispatch. `Smart` and `Retired` may share a wire id; that's
/// fine because we never deserialize this enum.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema, Default)]
pub enum PredefinedModel {
    /// Best available model (FORK: Kimi K3)
    #[default]
    #[serde(rename = "k3")]
    Smart,
    /// Fastest available model (FORK: Kimi K2.7 Code)
    #[serde(rename = "kimi-for-coding")]
    Fast,
    /// Claude Opus 4.7
    #[serde(rename = "claude-opus-4-7")]
    Opus4_7,
    /// Claude Sonnet 5
    #[serde(rename = "claude-sonnet-5")]
    Sonnet5,
    /// Mid tier for judges and summarizers (FORK: MiniMax M2.7)
    #[serde(rename = "MiniMax-M2.7")]
    Sonnet4_6,
    /// Fast tier alias (FORK: Kimi K2.7 Code)
    #[serde(rename = "kimi-for-coding")]
    Haiku4_5,
    /// OpenAI GPT-5.5
    #[serde(rename = "gpt-5.5")]
    Gpt5_5,
    /// OpenAI GPT-5 mini
    #[serde(rename = "gpt-5-mini")]
    Gpt5Mini,
    /// Retired or unrecognized model, routes to the default
    #[serde(rename = "k3")]
    Retired,
}

impl From<PredefinedModel> for super::types::Model<'static> {
    fn from(model: PredefinedModel) -> Self {
        let (provider, name) = match model {
            PredefinedModel::Smart | PredefinedModel::Retired => (KIMI, KIMI_K3),
            PredefinedModel::Opus4_7 => (ANTHROPIC, CLAUDE_OPUS_4_7),
            PredefinedModel::Sonnet5 => (ANTHROPIC, CLAUDE_SONNET_5),
            PredefinedModel::Sonnet4_6 => (MINIMAX, MINIMAX_M27),
            PredefinedModel::Fast | PredefinedModel::Haiku4_5 => (KIMI, KIMI_K27_CODE),
            PredefinedModel::Gpt5_5 => (OPENAI, GPT_5_5),
            PredefinedModel::Gpt5Mini => (OPENAI, GPT_5_MINI),
        };
        super::types::Model {
            provider: std::borrow::Cow::Borrowed(provider),
            name: std::borrow::Cow::Borrowed(name),
        }
    }
}

impl PredefinedModel {
    /// Returns `additional_params` JSON to enable extended thinking / reasoning.
    ///
    /// - Kimi K3 (`k3`): thinks by default on the Anthropic-compatible
    ///   surface; send nothing
    /// - Kimi K2.7 Code (`kimi-for-coding`): requires thinking enabled —
    ///   requests without it are rejected
    /// - MiniMax M2.7: reasoning cannot be disabled on the OpenAI-compatible
    ///   API; send nothing
    /// - Opus 4.7 / Sonnet 5: Anthropic `adaptive` (model chooses when to think)
    /// - GPT-5.5 / GPT-5 mini: Responses API `reasoning` with effort
    ///   (no `temperature`; reasoning models reject it)
    pub fn thinking_params(&self) -> serde_json::Value {
        match self {
            Self::Smart | Self::Retired | Self::Sonnet4_6 => serde_json::json!({}),
            Self::Fast | Self::Haiku4_5 => serde_json::json!({
                "thinking": { "type": "enabled", "budget_tokens": 10_000 }
            }),
            Self::Sonnet5 => serde_json::json!({
                "thinking": { "type": "adaptive", "display": "summarized" }
            }),
            Self::Opus4_7 => serde_json::json!({
                "thinking": { "type": "adaptive", "display": "summarized" },
                "temperature": 1
            }),
            Self::Gpt5_5 => serde_json::json!({
                "reasoning": { "effort": "medium", "summary": "auto" }
            }),
            Self::Gpt5Mini => serde_json::json!({
                "reasoning": { "effort": "low", "summary": "auto" }
            }),
        }
    }

    /// Context window size in tokens.
    pub fn context_window(&self) -> u64 {
        match self {
            // Kimi Code serves K3 / K2.7 Code at 256K (Allegretto members
            // unlock 1M on K3 — bump this if that's you).
            Self::Smart | Self::Retired | Self::Fast | Self::Haiku4_5 => 262_144,
            Self::Opus4_7 | Self::Sonnet5 => 1_000_000,
            Self::Sonnet4_6 => 204_800,
            Self::Gpt5_5 | Self::Gpt5Mini => 400_000,
        }
    }
}

impl std::fmt::Display for PredefinedModel {
    /// Displays the provider-qualified id (`provider/name`) the router routes on.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let model: Model = (*self).into();
        write!(f, "{}", model)
    }
}
