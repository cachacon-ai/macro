//! Draft `agent-runtime.v0` message types.
//!
//! The outer protocol messages implement the corresponding ACP SDK JSON-RPC
//! traits. [`Command`] is a request, while [`SystemEvent`] and [`AcpMessage`]
//! are notifications. This allows them to use the same connection and dispatch
//! machinery as ACP messages while keeping their distinct outer method names.

use agent_client_protocol::{
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, RawJsonRpcMessage,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::fmt::{self, Display, Formatter};

#[cfg(test)]
mod test;

/// JSON-RPC method used to send a system event.
pub const SYSTEM_EVENT_METHOD: &str = "system_event";

/// JSON-RPC method used to send a runtime command.
pub const COMMAND_METHOD: &str = "command";

/// JSON-RPC method used to carry an ACP message.
pub const ACP_METHOD: &str = "acp";

/// The kind of runtime or agent state transition reported to the Agent Service.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SystemEventName {
    /// An application-defined event name.
    Unknown(String),
}

impl SystemEventName {
    /// Return the event name used on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(name) => name,
        }
    }
}

impl Display for SystemEventName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for SystemEventName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SystemEventName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Ok(Self::Unknown(name))
    }
}

/// A runtime or agent state transition sent to the Agent Service.
#[derive(Debug, Clone, Serialize, JsonRpcNotification, PartialEq, schemars::JsonSchema)]
#[notification(method = "system_event")]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SystemEvent {
    /// Stable identifier reused when replaying this logical event.
    pub event_id: String,
    /// Monotonically increasing sequence within a runtime instance.
    pub sequence: u64,
    /// Typed event name with forward-compatible handling of unknown values.
    #[schemars(with = "String")]
    pub name: SystemEventName,
    /// UTC occurrence time formatted according to RFC 3339.
    pub occurred_at: String,
    /// Stable logical agent identifier for an agent-scoped event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Current agent-process identifier for an agent-scoped event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_instance_id: Option<String>,
    /// Event-specific JSON payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemEventWire {
    event_id: String,
    sequence: u64,
    name: SystemEventName,
    occurred_at: String,
    agent_id: Option<String>,
    agent_instance_id: Option<String>,
    payload: Option<Value>,
}

impl<'de> Deserialize<'de> for SystemEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SystemEventWire::deserialize(deserializer)?;
        if wire.agent_id.is_some() != wire.agent_instance_id.is_some() {
            return Err(serde::de::Error::custom(
                "agentId and agentInstanceId must both be present or both be absent",
            ));
        }
        Ok(Self {
            event_id: wire.event_id,
            sequence: wire.sequence,
            name: wire.name,
            occurred_at: wire.occurred_at,
            agent_id: wire.agent_id,
            agent_instance_id: wire.agent_instance_id,
            payload: wire.payload,
        })
    }
}

impl SystemEvent {
    /// Construct a runtime-scoped system event without a payload.
    #[must_use]
    pub fn new(
        event_id: impl Into<String>,
        sequence: u64,
        name: SystemEventName,
        occurred_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            sequence,
            name,
            occurred_at: occurred_at.into(),
            agent_id: None,
            agent_instance_id: None,
            payload: None,
        }
    }

    /// Target this event at one running agent instance.
    #[must_use]
    pub fn agent(
        mut self,
        agent_id: impl Into<String>,
        agent_instance_id: impl Into<String>,
    ) -> Self {
        self.agent_id = Some(agent_id.into());
        self.agent_instance_id = Some(agent_instance_id.into());
        self
    }

    /// Attach an event-specific JSON payload.
    #[must_use]
    pub fn payload(mut self, payload: impl Into<Value>) -> Self {
        self.payload = Some(payload.into());
        self
    }
}

/// The operation requested by the Agent Service.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CommandName {
    /// An application-defined command name.
    Unknown(String),
}

impl CommandName {
    /// Return the command name used on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(name) => name,
        }
    }
}

impl Display for CommandName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for CommandName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CommandName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Ok(Self::Unknown(name))
    }
}

/// An operation requested by the Agent Service and handled by the Agent Runtime.
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcRequest, PartialEq, schemars::JsonSchema)]
#[request(method = "command", response = CommandResult)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Command {
    /// Stable identifier reused when retrying this logical command.
    pub command_id: String,
    /// Typed command name with forward-compatible handling of unknown values.
    #[schemars(with = "String")]
    pub name: CommandName,
    /// Stable logical agent identifier for an agent-scoped command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Current agent-process identifier when the command targets one execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_instance_id: Option<String>,
    /// Command-specific JSON payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

impl Command {
    /// Construct a runtime-scoped command without a payload.
    #[must_use]
    pub fn new(command_id: impl Into<String>, name: CommandName) -> Self {
        Self {
            command_id: command_id.into(),
            name,
            agent_id: None,
            agent_instance_id: None,
            payload: None,
        }
    }

    /// Target this command at a logical agent.
    #[must_use]
    pub fn agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// Restrict this command to the current execution of its target agent.
    #[must_use]
    pub fn agent_instance(mut self, agent_instance_id: impl Into<String>) -> Self {
        self.agent_instance_id = Some(agent_instance_id.into());
        self
    }

    /// Attach a command-specific JSON payload.
    #[must_use]
    pub fn payload(mut self, payload: impl Into<Value>) -> Self {
        self.payload = Some(payload.into());
        self
    }
}

/// Result of executing a command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcResponse, PartialEq, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CommandResult {
    /// The command completed synchronously.
    Completed {
        /// Optional command-specific result value.
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
}

impl CommandResult {
    /// Construct a synchronous completion without a result value.
    #[must_use]
    pub fn completed() -> Self {
        Self::Completed { value: None }
    }

    /// Construct a synchronous completion with a result value.
    #[must_use]
    pub fn completed_with(value: impl Into<Value>) -> Self {
        Self::Completed {
            value: Some(value.into()),
        }
    }
}

/// One complete ACP JSON-RPC message routed to a running agent instance.
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcNotification, schemars::JsonSchema)]
#[notification(method = "acp")]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct AcpMessage {
    /// Unique identifier for this outer ACP delivery.
    pub message_id: String,
    /// Stable logical identifier of the target agent.
    pub agent_id: String,
    /// Identifier of the target agent process execution.
    pub agent_instance_id: String,
    /// Complete nested ACP JSON-RPC request, notification, or response.
    #[schemars(with = "Value")]
    pub message: RawJsonRpcMessage,
}

impl AcpMessage {
    /// Construct an ACP delivery for one running agent instance.
    #[must_use]
    pub fn new(
        message_id: impl Into<String>,
        agent_id: impl Into<String>,
        agent_instance_id: impl Into<String>,
        message: RawJsonRpcMessage,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            agent_id: agent_id.into(),
            agent_instance_id: agent_instance_id.into(),
            message,
        }
    }
}
