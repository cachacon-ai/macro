use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use macro_user_id::cowlike::CowLike;
use macro_user_id::user_id::MacroUserIdStr;
use uuid::Uuid;

use crate::domain::models::{CallError, CallWebhookEvent, EgressS3Config};
use crate::domain::ports::CallRtcClient;

use super::{build_voip_push_payloads, exclude_voip_recipients, extract_recording_key};

fn user(email: &'static str) -> MacroUserIdStr<'static> {
    MacroUserIdStr::try_from_email(email).unwrap()
}

struct MockRtcClient {
    tokens: Mutex<HashMap<String, anyhow::Result<String>>>,
    generate_calls: Mutex<Vec<(String, String)>>,
}

impl MockRtcClient {
    fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
            generate_calls: Mutex::new(Vec::new()),
        }
    }

    fn set_token(&self, identity: &str, token: anyhow::Result<String>) {
        self.tokens
            .lock()
            .unwrap()
            .insert(identity.to_string(), token);
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.generate_calls.lock().unwrap().clone()
    }
}

impl CallRtcClient for MockRtcClient {
    async fn create_room(&self, _room_name: &str) -> anyhow::Result<()> {
        unreachable!("create_room not exercised by these tests")
    }

    async fn delete_room(&self, _room_name: &str) -> anyhow::Result<()> {
        unreachable!("delete_room not exercised by these tests")
    }

    async fn generate_token<'a>(
        &self,
        room_name: &str,
        participant_identity: MacroUserIdStr<'a>,
    ) -> anyhow::Result<String> {
        let key = participant_identity.as_ref().to_string();
        self.generate_calls
            .lock()
            .unwrap()
            .push((room_name.to_string(), key.clone()));
        let mut tokens = self.tokens.lock().unwrap();
        tokens
            .remove(&key)
            .unwrap_or_else(|| Ok(format!("default-token-{key}")))
    }

    async fn remove_participant<'a>(
        &self,
        _room_name: &str,
        _participant_identity: MacroUserIdStr<'a>,
    ) -> anyhow::Result<()> {
        unreachable!("remove_participant not exercised by these tests")
    }

    async fn start_room_composite_egress(
        &self,
        _room_name: &str,
        _s3_config: &EgressS3Config,
    ) -> anyhow::Result<String> {
        unreachable!("start_room_composite_egress not exercised by these tests")
    }

    async fn stop_egress(&self, _egress_id: &str) -> anyhow::Result<()> {
        unreachable!("stop_egress not exercised by these tests")
    }

    fn receive_webhook(
        &self,
        _body: &str,
        _auth_token: &str,
    ) -> Result<CallWebhookEvent, CallError> {
        unreachable!("receive_webhook not exercised by these tests")
    }

    async fn dispatch_transcription_agent(&self, _room_name: &str) -> anyhow::Result<()> {
        unreachable!("dispatch_transcription_agent not exercised by these tests")
    }
}

#[test]
fn extract_key_from_full_s3_url() {
    let url = "https://macro-call-recording-prod.s3.amazonaws.com/calls/0195cea6-fc16-72f2-93b6-144df711f270/2026-04-10T210832.mp4";
    assert_eq!(
        extract_recording_key(url),
        "0195cea6-fc16-72f2-93b6-144df711f270/2026-04-10T210832.mp4"
    );
}

#[test]
fn extract_key_fallback_when_no_calls_prefix() {
    let url = "s3://bucket/some/other/path.mp4";
    assert_eq!(extract_recording_key(url), url);
}

#[test]
fn extract_key_from_bare_calls_path() {
    let url = "calls/abc-123/recording.mp4";
    assert_eq!(extract_recording_key(url), "abc-123/recording.mp4");
}

#[test]
fn exclude_voip_recipients_keeps_users_without_voip_delivery() {
    let alice = user("alice@example.com");
    let bob = user("bob@example.com");
    let recipients = HashSet::from([alice.clone(), bob.clone()]);
    let voip_recipients = HashSet::from([alice]);

    let filtered = exclude_voip_recipients(recipients, &voip_recipients);

    assert_eq!(filtered, HashSet::from([bob]));
}

#[test]
fn exclude_voip_recipients_returns_empty_when_all_users_received_voip() {
    let alice = user("alice@example.com");
    let bob = user("bob@example.com");
    let recipients = HashSet::from([alice.clone(), bob.clone()]);
    let voip_recipients = HashSet::from([alice, bob]);

    let filtered = exclude_voip_recipients(recipients, &voip_recipients);

    assert!(filtered.is_empty());
}

#[tokio::test]
async fn build_voip_push_payloads_mints_a_distinct_token_per_recipient() {
    let alice = user("alice@example.com").into_owned();
    let bob = user("bob@example.com").into_owned();
    let mock = MockRtcClient::new();
    mock.set_token(alice.as_ref(), Ok("token-alice".to_string()));
    mock.set_token(bob.as_ref(), Ok("token-bob".to_string()));

    let recipients = vec![alice.clone(), bob.clone()];
    let payloads = build_voip_push_payloads(
        &mock,
        &recipients,
        "room-1",
        Uuid::nil(),
        "channel-1",
        "general",
        "Carla",
        "wss://lk.example",
    )
    .await;

    assert_eq!(payloads.len(), 2);
    let by_id: HashMap<String, String> = payloads
        .into_iter()
        .map(|(id, p)| {
            (
                id.as_ref().to_string(),
                p.livekit_token.expect("livekit_token populated on success"),
            )
        })
        .collect();
    assert_eq!(by_id.get(alice.as_ref()).unwrap(), "token-alice");
    assert_eq!(by_id.get(bob.as_ref()).unwrap(), "token-bob");
    assert_eq!(mock.calls().len(), 2);
    for (room, _) in mock.calls() {
        assert_eq!(room, "room-1");
    }
}

#[tokio::test]
async fn build_voip_push_payloads_drops_recipients_whose_token_mint_fails() {
    let alice = user("alice@example.com").into_owned();
    let bob = user("bob@example.com").into_owned();
    let mock = MockRtcClient::new();
    mock.set_token(alice.as_ref(), Ok("token-alice".to_string()));
    mock.set_token(bob.as_ref(), Err(anyhow::anyhow!("livekit unreachable")));

    let recipients = vec![alice.clone(), bob.clone()];
    let payloads = build_voip_push_payloads(
        &mock,
        &recipients,
        "room-1",
        Uuid::nil(),
        "channel-1",
        "general",
        "Carla",
        "wss://lk.example",
    )
    .await;

    assert_eq!(
        payloads.len(),
        1,
        "bob's failed token mint should not block alice's payload"
    );
    let (id, payload) = &payloads[0];
    assert_eq!(id.as_ref(), alice.as_ref());
    assert_eq!(payload.livekit_token.as_deref(), Some("token-alice"));
}

#[tokio::test]
async fn build_voip_push_payloads_returns_empty_for_no_recipients() {
    let mock = MockRtcClient::new();
    let recipients: Vec<MacroUserIdStr<'static>> = Vec::new();

    let payloads = build_voip_push_payloads(
        &mock,
        &recipients,
        "room-1",
        Uuid::nil(),
        "channel-1",
        "general",
        "Carla",
        "wss://lk.example",
    )
    .await;

    assert!(payloads.is_empty());
    assert!(mock.calls().is_empty());
}
