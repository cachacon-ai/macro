//! Model availability for chat, gated by the user's plan.
//!
//! Free (non-professional) users may use only [`FREE_MODEL`]; professional
//! users may use every model in [`CHAT_MODELS`].
//!
//! FORK NOTE (BYOK): the offered list and defaults lead with the fork's BYOK
//! providers (Kimi, MiniMax). The Anthropic/OpenAI entries remain for
//! deployments that also configure those keys.

/// The chat models offered to users, best-first.
pub const CHAT_MODELS: &[&str] = &[
    "kimi/kimi-k3",
    "minimax/MiniMax-M3",
    "kimi/kimi-k2.7-code-highspeed",
    "minimax/MiniMax-M2.7-highspeed",
    "anthropic/claude-sonnet-5",
    "anthropic/claude-opus-4-8",
    "anthropic/claude-haiku-4-5",
    "anthropic/claude-opus-4-7",
    "anthropic/claude-sonnet-4-6",
    "openai/gpt-5.5",
    "openai/gpt-5-mini",
];

/// The default model for professional (paid) users.
pub const PAID_DEFAULT_MODEL: &str = "kimi/kimi-k3";

/// The only model available to free (non-professional) users.
pub const FREE_MODEL: &str = "minimax/MiniMax-M2.7-highspeed";
