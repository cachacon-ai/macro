use super::*;

const KIMI_K3: &str = "kimi/k3";
const KIMI_FAST: &str = "kimi/kimi-for-coding";
const MINIMAX_M3: &str = "minimax/MiniMax-M3";
const MINIMAX_FAST: &str = "minimax/MiniMax-M2.7-highspeed";
const HAIKU: &str = "anthropic/claude-haiku-4-5";
const SONNET_5: &str = "anthropic/claude-sonnet-5";
const OPUS: &str = "anthropic/claude-opus-4-8";
const OPUS_4_7: &str = "anthropic/claude-opus-4-7";
const SONNET_4_6: &str = "anthropic/claude-sonnet-4-6";
const GPT_5_5: &str = "openai/gpt-5.5";
const GPT_5_MINI: &str = "openai/gpt-5-mini";

#[test]
fn free_user_only_has_the_free_model() {
    let svc = ModelAccessServiceImpl;
    // FORK: the free tier is MiniMax M2.7 HighSpeed.
    assert_eq!(svc.best_model(false), MINIMAX_FAST);
    assert!(svc.has_access(false, MINIMAX_FAST));
    assert!(!svc.has_access(false, KIMI_K3));
    assert!(!svc.has_access(false, MINIMAX_M3));
    assert!(!svc.has_access(false, OPUS));
    assert!(!svc.has_access(false, SONNET_4_6));
    assert!(!svc.has_access(false, GPT_5_5));
}

#[test]
fn professional_user_has_everything() {
    let svc = ModelAccessServiceImpl;
    // FORK: the paid default is Kimi K3.
    assert_eq!(svc.best_model(true), KIMI_K3);
    assert!(svc.has_access(true, KIMI_K3));
    assert!(svc.has_access(true, KIMI_FAST));
    assert!(svc.has_access(true, MINIMAX_M3));
    assert!(svc.has_access(true, MINIMAX_FAST));
    assert!(svc.has_access(true, SONNET_5));
    assert!(svc.has_access(true, HAIKU));
    assert!(svc.has_access(true, OPUS));
    assert!(svc.has_access(true, OPUS_4_7));
    assert!(svc.has_access(true, SONNET_4_6));
    assert!(svc.has_access(true, GPT_5_5));
    assert!(svc.has_access(true, GPT_5_MINI));
}
