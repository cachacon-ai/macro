//! Draft `agent-runtime.v0` message types.
//!
//! The logical protocol multiplexes three kinds of traffic over one stream:
//! runtime/agent lifecycle events, application-defined commands, and ACP
//! conversation traffic. Each direction has its own envelope -
//! [`ToRuntimeMessage`] for Agent Service to Agent Runtime traffic and
//! [`ToServerMessage`] for the reverse - discriminated by a `"type"` field.
//! Only the wrapped ACP payload is itself a JSON-RPC message; the outer
//! envelope is not.
//!
//! A connection hosts exactly one agent execution, so nothing on this schema
//! needs an agent or agent-instance identifier: [`AcpMessage`] simply carries
//! the agent's raw ACP traffic, and [`CommandRequest`]/[`CommandOutcome`]
//! correlate through `command_id` alone.

use agent_client_protocol::RawJsonRpcMessage;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[cfg(test)]
mod test;

/// The kind of runtime or agent state transition reported to the Agent Service.
///
/// The protocol does not define an event-name catalog yet. This enum is
/// non-exhaustive and currently contains only [`SystemEvent::Unknown`]. Every
/// wire string round-trips through it unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SystemEvent {
    /// An application-defined event name.
    Unknown(String),
}

impl SystemEvent {
    /// Borrow the wire string for this event name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(name) => name,
        }
    }
}

impl Serialize for SystemEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SystemEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::Unknown(String::deserialize(deserializer)?))
    }
}

/// A well-typed operation for the Agent Runtime to execute, tagged by name.
///
/// The protocol does not define a command catalog yet. This enum is
/// non-exhaustive and currently contains only [`Command::Unknown`], which
/// carries the wire's opaque command-name string as its data. Every wire
/// string round-trips through it unchanged. In particular, this protocol
/// does not define commands for starting, stopping, or restarting agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "name", content = "payload", rename_all = "camelCase")]
#[non_exhaustive]
pub enum Command {
    /// An application-defined command not yet given its own variant.
    Unknown(String),
}

/// A [`Command`] correlated with its eventual outcome.
///
/// `command_id` correlates this request with its eventual outcome in
/// [`ToServerMessage::CommandResult`]. Commands can run concurrently and
/// complete out of order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CommandRequest {
    /// Correlates this request with its eventual outcome.
    pub command_id: String,
    /// The command to execute.
    #[serde(flatten)]
    pub command: Command,
}

impl CommandRequest {
    /// Construct a command request correlated by `command_id`.
    #[must_use]
    pub fn new(command_id: impl Into<String>, command: Command) -> Self {
        Self {
            command_id: command_id.into(),
            command,
        }
    }
}

/// The outcome of executing a [`Command`]: its result, or the error it failed
/// with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CommandOutcome {
    /// Matches the [`Command::command_id`] this outcome belongs to.
    pub command_id: String,
    /// The command's result, or the error it failed with.
    pub result: CommandResult,
}

impl CommandOutcome {
    /// Construct an outcome correlated to the command that produced it.
    #[must_use]
    pub fn new(command_id: impl Into<String>, result: CommandResult) -> Self {
        Self {
            command_id: command_id.into(),
            result,
        }
    }
}

/// The result of executing a [`Command`], or the error it failed with.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum CommandResult {
    /// The command completed synchronously.
    Completed {
        /// Optional command-specific result value.
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
    /// The command failed.
    Failed {
        /// A human-readable description of the failure.
        error: String,
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

    /// Construct a failed outcome.
    #[must_use]
    pub fn failed(error: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
        }
    }
}

/// One complete ACP JSON-RPC message routed between the Agent Service and the
/// single agent execution hosted by this connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpMessage(pub RawJsonRpcMessage);

/// Agent Service to Agent Runtime traffic on the logical protocol stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum ToRuntimeMessage {
    /// An ACP message routed to the hosted agent.
    Acp(AcpMessage),
    /// A command for the runtime to execute.
    Command(CommandRequest),
}

/// Agent Runtime to Agent Service traffic on the logical protocol stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum ToServerMessage {
    /// An ACP message routed from the hosted agent.
    Acp(AcpMessage),
    /// A runtime or agent lifecycle event.
    Event {
        /// The event name.
        event: SystemEvent,
    },
    /// The outcome of a previously issued [`Command`].
    CommandResult(CommandOutcome),
}
