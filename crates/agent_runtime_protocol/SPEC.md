# Agent Runtime Protocol

This is an internal design note for the protocol between our agent service and
the runtime inside a container. It is meant to answer practical questions:

- Which side connects?
- How does the service send commands back into the container?
- How do ACP messages get to the right agent process?
- Can we use the same code without a WebSocket?

## The problem

We want a container to make one outbound connection to our service and keep it
open for the lifetime of the container. The connection needs to carry three
different kinds of traffic:

| Message | Direction | What it is for |
|---|---|---|
| `SystemEvent` | Runtime → Service | Runtime and agent lifecycle updates. |
| `Command` | Service → Runtime | An application-defined operation for the runtime. |
| `ACP` | Both directions | The actual conversation between an ACP client and an agent. |

The runtime connection should survive agent restarts. An agent is one thing
running inside the runtime, not the owner of the runtime connection.

There are two layers:

1. The logical protocol is a stream of complete JSON-RPC messages:
   `system_event`, `command`, and `acp`.
2. The production WebSocket binding uses jsonrpsee to move those messages.

The logical layer does not care about WebSockets. In Rust, its transport
boundary is `agent_client_protocol::Channel`. Tests and local tools can connect
the service and runtime with `Channel::duplex()` and skip networking entirely.

The crate currently includes:

- an in-memory setup using `Channel::duplex()`;
- a jsonrpsee WebSocket carrier;
- no generic `Carrier` trait. `Channel` is the abstraction boundary instead.

## Why the WebSocket carrier looks a little weird

The runtime initiates the WebSocket, so it is the jsonrpsee client. A jsonrpsee
client can make requests and open subscriptions, but it cannot register a
handler for a new request initiated by the server.

That means the service cannot send a normal top-level JSON-RPC `command`
request directly to the runtime.

We solve this by tunnelling complete logical JSON-RPC messages:

- the runtime opens a subscription for service → runtime traffic;
- the service puts logical messages into subscription items;
- the runtime uses `send` for runtime → service traffic.

The nested command is still a real JSON-RPC request with its own ID. Its
response comes back as a complete nested JSON-RPC response. The carrier does
not reinterpret it.

## WebSocket carrier

The carrier has four method names:

| Method | Direction | Purpose |
|---|---|---|
| `subscribe` | Runtime → Service | Open the service-to-runtime stream. |
| `message` | Service → Runtime | Deliver a subscription item. |
| `unsubscribe` | Runtime → Service | Close the stream. |
| `send` | Runtime → Service | Deliver one logical message to the service. |

### Opening the connection

After opening the WebSocket, the runtime calls `subscribe` with a
`connectionId`. The runtime chooses this identifier, and it should be unique
to the connection attempt.

### Service to runtime

The service sends each logical message as a subscription item. A command has
two IDs with different purposes:

- `subscription` belongs to jsonrpsee;
- the nested JSON-RPC request ID belongs to the logical command.

Command responses are matched using the nested logical ID, not `commandId`.

### Runtime to service

The runtime carries each logical message with `send`, including the active
`connectionId` and the complete nested JSON-RPC message.

`send` returns `null` when the carrier accepted the nested message. That is not
an acknowledgement of the logical notification. It only tells the runtime
that the message reached the logical connection.

If `connectionId` has no active subscription, the service returns JSON-RPC
error `-32004`.

## Startup flow

The expected startup order is:

```text
Runtime                                      Service
   │                                            │
   ├── open WebSocket ─────────────────────────►│
   ├── subscribe(connectionId) ────────────────►│
   │◄────────────────────────── subscription ID ┤
   ├── system_event(runtime/ready) ────────────►│
   │◄──────────────── command(runtime/configure)┤
   ├── command response ───────────────────────►│
   │                                            │
   ├── start agent process                      │
   ├── system_event(agent/started) ────────────►│
   │◄────────────────────────── acp(initialize) ┤
   ├── acp(initialize response) ───────────────►│
```

The command and event names in this diagram are application-defined strings,
not protocol-defined enum variants. Starting the agent process is a local
runtime action; it is not caused by an agent-lifecycle command from the
service.

`runtime/ready` is the runtime's first logical message. The service normally
answers with `runtime/configure` before it sends other commands or ACP traffic.

The runtime opens the agent's ACP channel before emitting `agent/started`. If
startup fails, it emits `agent/stopped` or another useful diagnostic event for
the same Agent Instance ID.

## Logical messages

The logical stream has only three method names:

| Method | Direction | JSON-RPC shape |
|---|---|---|
| `system_event` | Runtime → Service | Notification |
| `command` | Service → Runtime | Request and response |
| `acp` | Both directions | Notification |

### SystemEvent

```typescript
interface SystemEvent {
  eventId: string;
  sequence: number;
  name: SystemEventName;
  occurredAt: string;
  agentId?: string;
  agentInstanceId?: string;
  payload?: unknown;
}
```

The protocol does not define an event-name catalog yet. The Rust enum is
non-exhaustive and currently contains only `SystemEventName::Unknown(String)`.
Every wire string is preserved in that variant.

An agent-scoped event includes both `agentId` and `agentInstanceId`. A
runtime-scoped event omits both.

`eventId` stays the same if the same logical event is replayed. `sequence`
increases within a runtime instance. `occurredAt` is an RFC 3339 timestamp.

System events are notifications. There is no logical response or acceptance
message.

### Command

```typescript
interface Command {
  commandId: string;
  name: CommandName;
  agentId?: string;
  agentInstanceId?: string;
  payload?: unknown;
}

type CommandResult = {
  status: "completed";
  value?: unknown;
};
```

The protocol does not define a command-name catalog yet. This enum is also
non-exhaustive and currently contains only `CommandName::Unknown(String)`.
Every wire string round-trips unchanged. In particular, this protocol does not
define commands for starting, stopping, or restarting agents.

A command is a JSON-RPC request, so it also has a logical request ID in the
JSON-RPC envelope. That ID is what matches the response. `commandId` is an
application-level identifier that handlers can use for logging or
deduplication.

There is no `accepted` result. A command either completes with
`CommandResult`, returns a JSON-RPC error, or remains pending until the
connection closes.

Commands can run concurrently and complete out of order.

### ACP

```typescript
interface AcpMessage {
  messageId: string;
  agentId: string;
  agentInstanceId: string;
  message: object;
}
```

`message` is one complete ACP `RawJsonRpcMessage`. It can be an ACP request,
notification, success response, or error response. We do not convert it to a
string or rewrite its request IDs.

The outer `acp` message is always a notification. If the nested ACP message is
a request, its response arrives later in another `acp` notification.

## Routing ACP to agents

One runtime connection can host several agent processes. ACP routes are keyed
by both `agentId` and `agentInstanceId`.

The instance ID matters because an agent may restart while keeping the same
logical Agent ID. A message for the old process should never reach the new one.

Unknown targets are discarded locally because the outer message is a
notification and has no response. Each failed delivery is traced at `TRACE`
level with its message, agent, and agent-instance identifiers.

The Rust API returns an official `agent_client_protocol::Channel` for each
target. This lets the service attach an official ACP `Client` and lets the
runtime attach an official ACP `Agent` or `AcpAgent` process wrapper directly.

## Connection lifecycle

The runtime opens a jsonrpsee WebSocket and calls `connect_runtime`. That
function subscribes before returning the logical `Channel`, which is then
passed to `RuntimeConnection::connect` with a `CommandHandler`.

Dropping either role connection stops its driver. Dropping the runtime also
cancels in-flight command handlers and closes its ACP channels. Pending service
commands fail instead of hanging forever.
