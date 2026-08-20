//! Model availability for chat, gated by the user's plan.
//!
//! Free (non-professional) users may use only [`FREE_MODEL`]; professional
//! users may use every model in [`CHAT_MODELS`].
//!
//! FORK NOTE (BYOK): the offered list and defaults lead with the fork's BYOK
//! providers — Kimi (Kimi Code endpoint ids `k3` / `kimi-for-coding`) and
//! MiniMax. The Anthropic/OpenAI entries remain for deployments that also
//! configure those keys.

/// The chat models offered to users, best-first.
pub const CHAT_MODELS: &[&str] = &[
    "kimi/k3",
    "minimax/MiniMax-M3",
    "kimi/kimi-for-coding",
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
/// FORK: this is a single-user fork — no paid concept, so the paid default is
/// the same MiniMax HS model as free. (The fork owner wanted "no paid anything
/// at all"; the picker still respects the plan gate elsewhere, but chat
/// defaults to MiniMax regardless.)
pub const PAID_DEFAULT_MODEL: &str = "minimax/MiniMax-M2.7-highspeed";

/// The only model available to free (non-professional) users.
pub const FREE_MODEL: &str = "minimax/MiniMax-M2.7-highspeed";
