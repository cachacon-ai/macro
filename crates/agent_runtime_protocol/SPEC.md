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
running inside the runtime, not the owner of the runtime connection. A
connection hosts exactly one agent execution: there is no routing table, and
no `agentId`/`agentInstanceId` anywhere in this schema.

There are two layers:

1. The logical protocol is a stream of typed messages, each tagged with a
   `"type"` field: `acp`, `command`, `event`, or `commandResult`. Unlike an
   ordinary JSON-RPC method dispatch, this outer envelope is not itself
   JSON-RPC - only the ACP payload it can carry is.
2. The production WebSocket binding uses jsonrpsee to move those messages.

The logical layer does not care about WebSockets. In Rust, its transport
boundary is [`crate::domain::channel::Channel`] - a plain pair of
`tokio::sync::mpsc` halves, generic over what each side sends and receives.
Unlike a jsonrpsee client or server, it depends on nothing but `tokio`, so it
can be exposed to consumers outside this crate. Tests and local tools can
connect the service and runtime with `Channel::duplex()` and skip networking
entirely.

The crate currently includes:

- an in-memory setup using `Channel::duplex()`;
- a jsonrpsee WebSocket carrier ([`crate::outbound::jsonrpsee`]);
- a `Transport<Tx, Rx>` port ([`crate::domain::ports`]) that physical carriers
  implement, and a private `pump` that bridges any `Transport` into a
  `Channel`.

ACP traffic is handled separately from the rest: `ServerConnection::connect`
and `RuntimeConnection::connect` each return an official
`agent_client_protocol::Channel` alongside the connection handle, for the one
agent execution this connection hosts. An attached ACP `Client` or `Agent`
talks directly to that channel; it never sees this crate's envelope types.

## Why the WebSocket carrier looks a little weird

The runtime initiates the WebSocket, so it is the jsonrpsee client. A jsonrpsee
client can make requests and open subscriptions, but it cannot register a
handler for a new request initiated by the server.

That means the service cannot send a normal top-level JSON-RPC `command`
request directly to the runtime.

We solve this by tunnelling complete logical messages:

- the runtime opens a subscription for service → runtime traffic;
- the service puts logical messages into subscription items;
- the runtime uses `send` for runtime → service traffic.

`send` itself depends on the subscription already existing - the server looks
up where to route a `send` by the physical connection's subscription, not by
anything the runtime supplies - so [`crate::outbound::jsonrpsee::JsonRpseeWire`]
guarantees the subscribe handshake happens before the first `send`, no matter
which of `Transport::send` or `Transport::recv` a caller happens to reach
first.

## WebSocket carrier

The carrier has four method names:

| Method | Direction | Purpose |
|---|---|---|
| `subscribe` | Runtime → Service | Open the service-to-runtime stream. |
| `message` | Service → Runtime | Deliver a subscription item. |
| `unsubscribe` | Runtime → Service | Close the stream. |
| `send` | Runtime → Service | Deliver one logical message to the service. |

Every physical payload in either direction is `{ "message": <logical message> }`,
where `<logical message>` is one complete `ToRuntimeMessage` or
`ToServerMessage` value, `"type"`-tagged as described below.

### Opening the connection

After opening the WebSocket, the runtime calls `subscribe` with no parameters.
One physical connection is exactly one logical session: the server keys the
subscription by its own internal connection identity, not by anything the
runtime supplies. There is no way to multiplex several logical sessions over
one socket, and no session resumption across a reconnect — a new physical
connection is always a new logical session.

### Runtime to service

The runtime carries each logical message with `send`, wrapping it as
`{ "message": <ToServerMessage> }`. `send` returns `null` once the carrier has
accepted the nested message onto the logical connection. That is not an
acknowledgement of anything the message itself asked for - a `command`'s
result, for instance, still arrives later as its own `commandResult` message.

If this physical connection has no active subscription, the service returns
JSON-RPC error `-32004`.

### Service to runtime

The service sends each logical message as a subscription item, wrapping it the
same way: `{ "message": <ToRuntimeMessage> }`.

## Startup flow

The expected startup order is:

```text
Runtime                                      Service
   │                                            │
   ├── open WebSocket ─────────────────────────►│
   ├── subscribe() ─────────────────────────────►│
   │◄────────────────────────── subscription ID ┤
   ├── event(runtime/ready) ───────────────────►│
   │◄────────────────────────── command(runtime/configure)┤
   ├── commandResult ──────────────────────────►│
   │                                            │
   ├── start agent process                      │
   ├── event(agent/started) ────────────────────►│
   │◄────────────────────────────── acp(initialize) ┤
   ├── acp(initialize response) ───────────────►│
```

The event and command names in this diagram are application-defined wire
strings, not protocol-defined enum variants. Starting the agent process is a
local runtime action; it is not caused by a command from the service.

`runtime/ready` is the runtime's first logical message. The service normally
answers with a `runtime/configure` command before it sends other commands or
ACP traffic.

The runtime opens the agent's ACP channel before emitting `agent/started`. If
startup fails, it emits `agent/stopped` or another useful diagnostic event.

## Logical messages

The logical stream has one envelope per direction, each `"type"`-tagged:

| Type | Direction | Carries |
|---|---|---|
| `event` | Runtime → Service | A [`SystemEvent`] |
| `command` | Service → Runtime | A [`CommandRequest`] |
| `commandResult` | Runtime → Service | A [`CommandOutcome`] |
| `acp` | Both directions | An [`AcpMessage`] |

### SystemEvent

The protocol does not define an event-name catalog yet. The Rust enum is
non-exhaustive and currently contains only `SystemEvent::Unknown(String)`.
Every wire string is preserved in that variant, and it serializes as a bare
JSON string:

```json
{ "type": "event", "event": "agent/stopped" }
```

System events are fire-and-forget: there is no logical response or
acceptance message.

### Command / CommandRequest

`Command` is itself a `"name"`/`"payload"`-tagged enum - not a struct with an
adjacent name field and an untyped JSON value. The protocol does not define a
command catalog yet, so it currently has one placeholder variant,
`Command::Unknown(String)`, carrying the wire's opaque command name as its
data:

```json
{ "name": "unknown", "payload": "runtime/configure" }
```

As real commands are defined, each becomes its own tagged variant with its own
well-typed fields - `payload` stops being generic once that happens.

`CommandRequest` pairs a `Command` with the `command_id` that correlates it to
its eventual [`CommandOutcome`]:

```json
{ "type": "command", "commandId": "019c-command", "name": "unknown", "payload": "runtime/configure" }
```

Commands can run concurrently and complete out of order; `command_id` is the
only thing that ties a request to its outcome, since there is no outer
JSON-RPC envelope to carry a request ID anymore. In particular, this protocol
does not define commands for starting, stopping, or restarting agents.

### CommandOutcome

```json
{
  "type": "commandResult",
  "commandId": "019c-command",
  "result": { "type": "completed", "value": { "handled": true } }
}
```

`result` is a `CommandResult`, tagged `completed` (with an optional `value`)
or `failed` (with a human-readable `error` string). A command handler that
panics is reported as a `failed` outcome, not a crashed connection.

### ACP

`AcpMessage` wraps one complete ACP `RawJsonRpcMessage` - it can be an ACP
request, notification, success response, or error response. We do not convert
it to a string or rewrite its request IDs, and we do not attach any
agent/instance identifier to it: a connection hosts exactly one agent
execution, so there is nothing to route between.

```json
{
  "type": "acp",
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": { "protocolVersion": 1 }
}
```

## Connection lifecycle

`ServerConnection::connect` and `RuntimeConnection::connect` each take a
[`crate::domain::channel::Channel`] and a handler (`SystemEventHandler` /
`CommandHandler`, or `()` for a connection that only carries ACP), and return
`(connection, acp_channel)`. Dropping the connection handle stops its driver
task and, for the runtime side, cancels in-flight command handlers.

The two ACP channel and command/event driving pumps run independently:
dropping the ACP channel (because a caller only cares about commands/events)
stops that connection from carrying ACP traffic, but does not tear down
command or event handling - only an actual transport failure does that.

For the WebSocket binding, the runtime opens a jsonrpsee `WsClient` and calls
[`crate::outbound::jsonrpsee::connect_runtime`], which wraps a
[`crate::outbound::jsonrpsee::JsonRpseeWire`] in the crate-private `pump` and
returns the logical `Channel` immediately - the subscribe handshake happens
lazily, the first time either `send` or `recv` needs it. That `Channel` is
then passed to `RuntimeConnection::connect`.
