//! Model routing.
//!
//! Routing turns a `provider/model` api id into a runnable agent and owns the
//! provider fan-out so the rest of the crate stays provider-agnostic:
//!
//! - [`RoutedModel`] — the routed id bound to its provider client. One arm per
//!   wire protocol: Anthropic-native, OpenAI Responses, and OpenAI-compatible
//!   Chat Completions. Compatible providers live in a data registry keyed by
//!   name, so adding one is [`with_openai_provider`](ModelRouter::with_openai_provider).
//! - [`ProviderAgent`] — a built rig agent, with the same arms. Its
//!   [`run_stream`](ProviderAgent::run_stream) matches internally, so callers
//!   (e.g. `agent_loop`) hold one type and never fan out.
//!
//! Ids are addressed as `provider/model` (e.g. `anthropic/claude-opus-4-8`,
//! `kimi/kimi-k3`); routing picks the provider from the segment, never by
//! sniffing the id. Unroutable ids fall back to the default model.
//!
//! FORK NOTE (BYOK): every provider is optional. The router is built from
//! whichever keys are present in the environment — built-ins
//! (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `CEREBRAS_API_KEY`) plus BYOK
//! providers (`KIMI_API_KEY`/`KIMI_BASE_URL`, `MINIMAX_API_KEY`/`MINIMAX_BASE_URL`).
//! Ids whose provider is not configured are unroutable and fall back to the
//! default model.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use ai_toolset::{RequestContext, SearchableTool};
use ai_usage::{UsageContext, UsageRecorder};
use futures::StreamExt;
use rig_core::agent::{Agent, AgentBuilder, MultiTurnStreamItem};
use rig_core::completion::{CompletionModel, GetTokenUsage};
use rig_core::message::Message;
use rig_core::providers::anthropic::completion::CLAUDE_OPUS_4_8;
use rig_core::providers::openai::GPT_5_5;
use rig_core::providers::{anthropic, openai};
use rig_core::streaming::{StreamedAssistantContent, StreamingPrompt};
use rig_core::tool::server::ToolServerHandle;

use super::PredefinedModel;
use super::anthropic::AnthropicModel;
use super::openai::{OpenAiChatCompletionsModel, OpenAiResponsesModel};
use super::types::Model;
use crate::error::AgentError;
use crate::hook::{RegisterFn, StreamBridge, ToolRouter};
use crate::stream::{ChatCompletionStream, StreamPart};

/// Provider segment for native Anthropic.
const ANTHROPIC_PROVIDER: &str = "anthropic";
/// Provider segment the built-in OpenAI client is registered under.
const OPENAI_PROVIDER: &str = "openai";
/// Provider segment Cerebras is registered under (OpenAI-compatible Chat
/// Completions).
const CEREBRAS_PROVIDER: &str = "cerebras";
/// Cerebras inference endpoint (OpenAI-compatible Chat Completions API).
const CEREBRAS_BASE_URL: &str = "https://api.cerebras.ai/v1";
/// Provider segment Kimi is registered under (OpenAI-compatible Chat
/// Completions). FORK: BYOK provider.
const KIMI_PROVIDER: &str = "kimi";
/// Default Kimi endpoint: the pay-as-you-go Kimi Platform API. Override with
/// `KIMI_BASE_URL` — e.g. `https://api.kimi.com/coding/v1` for a Kimi Code
/// subscription key (note: Kimi restricts that endpoint to approved
/// coding-agent clients; the Platform endpoint has no such restriction).
const KIMI_DEFAULT_BASE_URL: &str = "https://api.moonshot.ai/v1";
/// Provider segment MiniMax is registered under (OpenAI-compatible Chat
/// Completions). FORK: BYOK provider.
const MINIMAX_PROVIDER: &str = "minimax";
/// Default MiniMax endpoint (international OpenAI-compatible API). Override
/// with `MINIMAX_BASE_URL`.
const MINIMAX_DEFAULT_BASE_URL: &str = "https://api.minimax.io/v1";

/// A routed model id bound to the provider client that serves it.
pub(crate) enum RoutedModel<'a> {
    /// A model on Anthropic's native API.
    Anthropic(AnthropicModel<'a>),
    /// A model on the OpenAI-compatible Chat Completions API.
    OpenAiChatCompletions(OpenAiChatCompletionsModel<'a>),
    /// A model on OpenAI's Responses API.
    OpenAiResponses(OpenAiResponsesModel<'a>),
}

impl<'a> RoutedModel<'a> {
    /// Build the rig agent for this model, applying provider-specific thinking
    /// config. Pure construction — no model call is made here.
    pub(crate) fn into_agent(
        self,
        handle: ToolServerHandle,
        system_prompt: &str,
        max_turns: usize,
        max_tokens: u64,
    ) -> ProviderAgent {
        match self {
            RoutedModel::Anthropic(m) => {
                let thinking = m.thinking_params();
                ProviderAgent::Anthropic(build_agent(
                    m.completion(),
                    thinking,
                    handle,
                    system_prompt,
                    max_turns,
                    max_tokens,
                ))
            }
            RoutedModel::OpenAiChatCompletions(m) => {
                let thinking = m.thinking_params();
                ProviderAgent::OpenAiChatCompletions(build_agent(
                    m.completion(),
                    thinking,
                    handle,
                    system_prompt,
                    max_turns,
                    max_tokens,
                ))
            }
            RoutedModel::OpenAiResponses(m) => {
                let thinking = m.thinking_params();
                ProviderAgent::OpenAiResponses(build_agent(
                    m.completion(),
                    thinking,
                    handle,
                    system_prompt,
                    max_turns,
                    max_tokens,
                ))
            }
        }
    }
}

/// A built rig agent bound to the provider serving the session's model.
///
/// The two arms are different concrete `Agent<M>` types; [`run_stream`] hides
/// that behind one concrete [`ChatCompletionStream`], so callers never match.
///
/// [`run_stream`]: ProviderAgent::run_stream
pub(crate) enum ProviderAgent {
    /// An agent over Anthropic's native completion model.
    Anthropic(Agent<anthropic::completion::CompletionModel>),
    /// An agent over the OpenAI Chat Completions model.
    OpenAiChatCompletions(Agent<openai::completion::CompletionModel>),
    /// An agent over the OpenAI Responses model.
    OpenAiResponses(Agent<openai::responses_api::ResponsesCompletionModel>),
    /// A test-only agent over an arbitrary completion model (e.g. a scripted
    /// fake), type-erased so the enum itself stays non-generic.
    #[cfg(test)]
    Test(Box<dyn DynStreamAgent>),
}

impl ProviderAgent {
    /// Run the agentic loop and adapt rig's stream into the provider-agnostic
    /// [`StreamPart`] stream consumed by DCS. The provider fan-out is internal.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_stream(
        &self,
        prompt: Message,
        history: Vec<Message>,
        max_turns: usize,
        routing: ToolRouter,
        loaded_buffer: Arc<Mutex<Vec<SearchableTool>>>,
        register_loaded: RegisterFn,
        recorder: Arc<dyn UsageRecorder>,
        usage_ctx: UsageContext,
        model: String,
        request_context: RequestContext,
    ) -> ChatCompletionStream<'static> {
        match self {
            ProviderAgent::Anthropic(agent) => {
                drive_stream(
                    agent,
                    prompt,
                    history,
                    max_turns,
                    routing,
                    loaded_buffer,
                    register_loaded,
                    recorder,
                    usage_ctx,
                    model,
                    request_context.clone(),
                )
                .await
            }
            ProviderAgent::OpenAiChatCompletions(agent) => {
                drive_stream(
                    agent,
                    prompt,
                    history,
                    max_turns,
                    routing,
                    loaded_buffer,
                    register_loaded,
                    recorder,
                    usage_ctx,
                    model,
                    request_context.clone(),
                )
                .await
            }
            ProviderAgent::OpenAiResponses(agent) => {
                drive_stream(
                    agent,
                    prompt,
                    history,
                    max_turns,
                    routing,
                    loaded_buffer,
                    register_loaded,
                    recorder,
                    usage_ctx,
                    model,
                    request_context.clone(),
                )
                .await
            }
            #[cfg(test)]
            ProviderAgent::Test(agent) => {
                agent
                    .run_stream_dyn(
                        prompt,
                        history,
                        max_turns,
                        routing,
                        loaded_buffer,
                        register_loaded,
                        recorder,
                        usage_ctx,
                        model,
                        request_context.clone(),
                    )
                    .await
            }
        }
    }
}

/// Routes model api-id strings to the provider client that serves them.
///
/// Holds optional native Anthropic and OpenAI Responses clients plus a
/// registry of OpenAI-compatible Chat Completions clients keyed by provider
/// name. Register compatible providers with
/// [`with_openai_provider`](Self::with_openai_provider).
///
/// FORK: the built-in clients are `Option`s — a deployment can run on BYOK
/// providers (Kimi, MiniMax) alone, with no Anthropic/OpenAI key present.
#[derive(Clone)]
pub struct ModelRouter {
    anthropic: Option<Arc<anthropic::Client>>,
    openai: Option<Arc<openai::Client>>,
    openai_compatible: HashMap<String, Arc<openai::CompletionsClient>>,
}

impl ModelRouter {
    /// An empty router with no providers registered yet. Chain the `with_*`
    /// methods to add providers.
    pub fn empty() -> Self {
        Self {
            anthropic: None,
            openai: None,
            openai_compatible: HashMap::new(),
        }
    }

    /// Build a router over native Anthropic and OpenAI Responses clients, with
    /// no OpenAI-compatible Chat Completions providers registered yet.
    pub fn new(anthropic: anthropic::Client, openai: openai::Client) -> Self {
        Self {
            anthropic: Some(Arc::new(anthropic)),
            openai: Some(Arc::new(openai)),
            openai_compatible: HashMap::new(),
        }
    }

    /// Register the native Anthropic client from an API key.
    pub fn with_anthropic_key(mut self, api_key: impl Into<String>) -> Result<Self, AgentError> {
        let client = anthropic::Client::builder()
            .api_key(api_key.into())
            .build()?;
        self.anthropic = Some(Arc::new(client));
        Ok(self)
    }

    /// Register the native OpenAI Responses client from an API key.
    pub fn with_openai_key(mut self, api_key: impl Into<String>) -> Result<Self, AgentError> {
        let client = openai::Client::builder()
            .api_key(api_key.into())
            .build()?;
        self.openai = Some(Arc::new(client));
        Ok(self)
    }

    /// Read an env var, treating unset and empty as absent.
    fn env_key(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    /// Build a router from the environment, registering every provider whose
    /// key is present. All providers are optional — but at least one must be
    /// set or the router panics on first use (see
    /// [`default_model`](Self::default_model)).
    ///
    /// Built-ins: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `CEREBRAS_API_KEY`.
    /// BYOK: `KIMI_API_KEY` (+ optional `KIMI_BASE_URL`) and `MINIMAX_API_KEY`
    /// (+ optional `MINIMAX_BASE_URL`).
    pub fn try_from_env() -> Result<Self, AgentError> {
        let mut router = Self::empty();
        if let Some(key) = Self::env_key("ANTHROPIC_API_KEY") {
            router = router.with_anthropic_key(key)?;
        }
        if let Some(key) = Self::env_key("OPENAI_API_KEY") {
            router = router.with_openai_key(key)?;
        }
        if let Some(key) = Self::env_key("CEREBRAS_API_KEY") {
            // Cerebras speaks the OpenAI Chat Completions API, so it rides the
            // compatible-provider registry: `cerebras/<model>` ids route to it.
            router = router.with_openai_provider(CEREBRAS_PROVIDER, CEREBRAS_BASE_URL, &key)?;
        }
        // FORK: BYOK providers. Kimi defaults to the Kimi Platform endpoint;
        // set KIMI_BASE_URL to use a different gateway (or the Kimi Code
        // endpoint, subject to Kimi's coding-agent client restrictions).
        if let Some(key) = Self::env_key("KIMI_API_KEY") {
            let base_url =
                Self::env_key("KIMI_BASE_URL").unwrap_or_else(|| KIMI_DEFAULT_BASE_URL.to_string());
            router = router.with_openai_provider(KIMI_PROVIDER, &base_url, &key)?;
        }
        if let Some(key) = Self::env_key("MINIMAX_API_KEY") {
            let base_url = Self::env_key("MINIMAX_BASE_URL")
                .unwrap_or_else(|| MINIMAX_DEFAULT_BASE_URL.to_string());
            router = router.with_openai_provider(MINIMAX_PROVIDER, &base_url, &key)?;
        }
        Ok(router)
    }

    /// The process-wide full router, built from the environment on first use.
    ///
    /// This is the only router the crate uses — every entry point routes through
    /// the same fully-populated instance, so a model id resolves identically
    /// everywhere. Register additional OpenAI-compatible providers here as they
    /// are added.
    pub(crate) fn shared() -> Result<&'static ModelRouter, AgentError> {
        static ROUTER: OnceLock<ModelRouter> = OnceLock::new();
        if let Some(router) = ROUTER.get() {
            return Ok(router);
        }
        let router = Self::try_from_env()?;
        Ok(ROUTER.get_or_init(|| router))
    }

    /// Register an already-built OpenAI-compatible Chat Completions client under
    /// `provider`.
    pub fn with_openai_client(
        mut self,
        provider: impl Into<String>,
        client: openai::CompletionsClient,
    ) -> Self {
        self.openai_compatible
            .insert(provider.into(), Arc::new(client));
        self
    }

    /// Register an OpenAI-compatible Chat Completions provider from a base URL
    /// and key.
    ///
    /// This is the whole cost of adding a provider — models served by it are
    /// then reachable as `provider/<model-id>`. The extension point for the
    /// open provider set (Cerebras is wired this way in [`try_from_env`]).
    ///
    /// [`try_from_env`]: Self::try_from_env
    pub fn with_openai_provider(
        self,
        provider: impl Into<String>,
        base_url: &str,
        api_key: &str,
    ) -> Result<Self, AgentError> {
        let client = openai::CompletionsClient::builder()
            .api_key(api_key)
            .base_url(base_url)
            .build()?;
        Ok(self.with_openai_client(provider, client))
    }

    /// Route + build the agent in one step, falling back to the default model on
    /// an unroutable id.
    pub(crate) fn agent(
        &self,
        model: &str,
        handle: ToolServerHandle,
        system_prompt: &str,
        max_turns: usize,
        max_tokens: u64,
    ) -> ProviderAgent {
        self.route_or_default(model)
            .into_agent(handle, system_prompt, max_turns, max_tokens)
    }

    /// Route a `provider/model` id to the provider that serves it.
    ///
    /// Returns [`AgentError::UnknownModel`] if no provider claims it (and
    /// [`AgentError::MalformedModel`] if the id has no `provider/` segment).
    pub(crate) fn route<'a>(&self, model: &'a str) -> Result<RoutedModel<'a>, AgentError> {
        let parsed = Model::try_from(model)?;
        self.route_model(parsed)
    }

    /// Route a parsed model to the provider that serves it.
    fn route_model<'a>(&self, parsed: Model<'a>) -> Result<RoutedModel<'a>, AgentError> {
        if parsed.provider() == ANTHROPIC_PROVIDER {
            let client = self
                .anthropic
                .clone()
                .ok_or_else(|| AgentError::UnknownModel(parsed.to_string()))?;
            return Ok(RoutedModel::Anthropic(AnthropicModel::new(parsed, client)));
        }
        if parsed.provider() == OPENAI_PROVIDER {
            let client = self
                .openai
                .clone()
                .ok_or_else(|| AgentError::UnknownModel(parsed.to_string()))?;
            return Ok(RoutedModel::OpenAiResponses(OpenAiResponsesModel::new(
                parsed, client,
            )));
        }
        if let Some(client) = self.openai_compatible.get(parsed.provider()) {
            let client = Arc::clone(client);
            return Ok(RoutedModel::OpenAiChatCompletions(
                OpenAiChatCompletionsModel::new(parsed, client),
            ));
        }
        Err(AgentError::UnknownModel(parsed.to_string()))
    }

    /// Route `model`, falling back to the default model on an unroutable id.
    pub(crate) fn route_or_default<'a>(&self, model: &'a str) -> RoutedModel<'a> {
        self.route(model).unwrap_or_else(|_| self.default_model())
    }

    /// The fallback model: [`PredefinedModel::Smart`] routed through the
    /// registry.
    ///
    /// FORK: `Smart` is remapped to `kimi/kimi-k3` (see `predefined_model.rs`),
    /// so the default works with no Anthropic key present. If the Smart
    /// provider is not configured either, fall back to the built-in Anthropic
    /// then OpenAI clients; panic only when the deployment has no providers
    /// at all.
    fn default_model(&self) -> RoutedModel<'static> {
        let smart: Model<'static> = PredefinedModel::Smart.into();
        if let Ok(routed) = self.route_model(smart) {
            return routed;
        }
        if let Some(client) = &self.anthropic {
            return RoutedModel::Anthropic(AnthropicModel::new(
                Model {
                    provider: Cow::Borrowed(ANTHROPIC_PROVIDER),
                    name: Cow::Borrowed(CLAUDE_OPUS_4_8),
                },
                client.clone(),
            ));
        }
        if let Some(client) = &self.openai {
            return RoutedModel::OpenAiResponses(OpenAiResponsesModel::new(
                Model {
                    provider: Cow::Borrowed(OPENAI_PROVIDER),
                    name: Cow::Borrowed(GPT_5_5),
                },
                client.clone(),
            ));
        }
        panic!(
            "ModelRouter: no providers configured — set at least one of \
             KIMI_API_KEY, MINIMAX_API_KEY, ANTHROPIC_API_KEY, OPENAI_API_KEY, \
             CEREBRAS_API_KEY"
        )
    }
}

/// Build a rig agent from a completion model and per-session config.
fn build_agent<M: CompletionModel>(
    model: M,
    thinking: Option<serde_json::Value>,
    handle: ToolServerHandle,
    system_prompt: &str,
    max_turns: usize,
    max_tokens: u64,
) -> Agent<M> {
    let mut builder = AgentBuilder::new(model)
        .tool_server_handle(handle)
        .default_max_turns(max_turns)
        .max_tokens(max_tokens)
        .preamble(system_prompt);
    if let Some(params) = thinking {
        builder = builder.additional_params(params);
    }
    builder.build()
}

/// Run the agentic loop on `agent` and adapt rig's stream into the
/// provider-agnostic [`StreamPart`] stream consumed by DCS.
#[allow(clippy::too_many_arguments)]
async fn drive_stream<M>(
    agent: &Agent<M>,
    prompt: Message,
    history: Vec<Message>,
    max_turns: usize,
    routing: ToolRouter,
    loaded_buffer: Arc<Mutex<Vec<SearchableTool>>>,
    register_loaded: RegisterFn,
    recorder: Arc<dyn UsageRecorder>,
    usage_ctx: UsageContext,
    model: String,
    request_context: RequestContext,
) -> ChatCompletionStream<'static>
where
    M: CompletionModel + 'static,
    M::StreamingResponse: GetTokenUsage + Send + Sync,
{
    let (bridge, mut rx) = StreamBridge::channel(
        routing,
        loaded_buffer,
        register_loaded,
        request_context.searchable_tools.clone(),
        request_context.cancel.clone(),
    );
    // Driver-side sender for parts derived from rig stream items (thinking,
    // usage, errors). The lifecycle hooks (text, tool call, tool response) send
    // through their own clone inside `bridge`; both feed the same FIFO channel.
    let driver_tx = bridge.sender();

    let mut rig_stream = agent
        .stream_prompt(prompt)
        .with_history(history)
        .multi_turn(max_turns)
        .max_invalid_tool_call_retries(crate::hook::MAX_INVALID_TOOL_CALL_RETRIES)
        .with_hook(bridge)
        .await;

    // Drive the rig stream on its own task. The hook emits a tool call the
    // moment the model finishes it — *before* the (often slow) tool executes —
    // but rig runs that execution inside a single `rig_stream.next()` poll, so
    // draining the channel only between polls would hold the pending tool call
    // hidden until its response landed. Polling the rig stream here, off the
    // consumer's path, lets every hook-emitted part flow through `rx` and out
    // to the client as soon as it is produced — so a tool call renders in its
    // pending state immediately and its response renders when execution
    // finishes.
    let driver = tokio::spawn(async move {
        let mut thinking_buf = String::new();

        while let Some(item) = rig_stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. },
                )) => {
                    thinking_buf.push_str(&reasoning);
                }
                other => {
                    if !thinking_buf.is_empty() {
                        let _ = driver_tx
                            .send(Ok(StreamPart::Thinking(std::mem::take(&mut thinking_buf))));
                    }
                    match other {
                        Ok(MultiTurnStreamItem::FinalResponse(final_resp)) => {
                            let usage = final_resp.usage();
                            // Best-effort cost logging; never fails the stream.
                            recorder.record(usage_ctx.clone().into_event(
                                model.clone(),
                                usage.input_tokens,
                                usage.output_tokens,
                            ));
                            let _ = driver_tx.send(Ok(StreamPart::Usage(crate::stream::Usage {
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                            })));
                        }
                        Err(e) => {
                            let _ = driver_tx.send(Err(AgentError::Streaming(e)));
                        }
                        _ => {}
                    }
                }
            }
        }
        if !thinking_buf.is_empty() {
            let _ = driver_tx.send(Ok(StreamPart::Thinking(std::mem::take(&mut thinking_buf))));
        }
        // Dropping `rig_stream` (and with it the hook's sender) plus `driver_tx`
        // here closes the channel, ending the consumer stream below.
    });

    // Abort the driver when the consumer drops the returned stream (e.g. on
    // cancellation), which drops `rig_stream` and cancels any in-flight tool —
    // matching the prior behaviour where the rig stream lived inline.
    struct AbortOnDrop(tokio::task::JoinHandle<()>);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let guard = AbortOnDrop(driver);

    let stream = async_stream::stream! {
        let _guard = guard;
        while let Some(part) = rx.recv().await {
            yield part;
        }
    };

    Box::pin(stream)
}

/// Test-only type erasure so [`ProviderAgent`] can hold an arbitrary
/// [`Agent<M>`] (e.g. a scripted fake model) without the enum becoming generic.
/// Mirrors the production arms: it just drives [`drive_stream`].
#[cfg(test)]
pub(crate) trait DynStreamAgent: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn run_stream_dyn<'a>(
        &'a self,
        prompt: Message,
        history: Vec<Message>,
        max_turns: usize,
        max_tokens: u64,
        routing: ToolRouter,
        loaded_buffer: Arc<Mutex<Vec<SearchableTool>>>,
        register_loaded: RegisterFn,
        recorder: Arc<dyn UsageRecorder>,
        usage_ctx: UsageContext,
        model: String,
        request_context: RequestContext,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = ChatCompletionStream<'static>> + Send + 'a>,
    >;
}

#[cfg(test)]
impl<M> DynStreamAgent for Agent<M>
where
    M: CompletionModel + 'static,
    M::StreamingResponse: GetTokenUsage + Send + Sync,
{
    fn run_stream_dyn<'a>(
        &'a self,
        prompt: Message,
        history: Vec<Message>,
        max_turns: usize,
        max_tokens: u64,
        routing: ToolRouter,
        loaded_buffer: Arc<Mutex<Vec<SearchableTool>>>,
        register_loaded: RegisterFn,
        recorder: Arc<dyn UsageRecorder>,
        usage_ctx: UsageContext,
        model: String,
        request_context: RequestContext,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = ChatCompletionStream<'static>> + Send + 'a>,
    > {
        Box::pin(drive_stream(
            model,
            prompt,
            history,
            max_turns,
            routing,
            loaded_buffer,
            register_loaded,
            recorder,
            usage_ctx,
            model,
            request_context,
        ))
    }
}

#[cfg(test)]
impl ProviderAgent {
    /// Build a test-only [`ProviderAgent`] backed by `model` (a fake completion
    /// model), wired through the same [`build_agent`] used in production.
    pub(crate) fn test<M>(
        model: M,
        system_prompt: &str,
        max_turns: usize,
        max_tokens: u64,
        handle: ToolServerHandle,
    ) -> Self
    where
        M: CompletionModel + 'static,
        M::StreamingResponse: GetTokenUsage + Send + Sync,
    {
        ProviderAgent::Test(Box::new(build_agent(
            model,
            None,
            max_turns,
            max_tokens,
            handle,
        )))
    }
}
