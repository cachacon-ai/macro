use agent_client_protocol::schema::v1::RequestId;
use agent_client_protocol::{
    JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, RawJsonRpcMessage,
};
use serde_json::json;

use super::*;

fn assert_request_response_pair<Request, Response>()
where
    Request: JsonRpcRequest<Response = Response>,
    Response: JsonRpcResponse,
{
}

fn assert_notification<Notification: JsonRpcNotification>() {}

#[test]
fn request_types_use_the_acp_jsonrpc_traits() {
    assert_request_response_pair::<Command, CommandResult>();
    assert_notification::<SystemEvent>();
    assert_notification::<AcpMessage>();

    assert_eq!(
        SystemEvent::new(
            "event-1",
            1,
            SystemEventName::Unknown("runtime/ready".to_owned()),
            "2026-07-17T00:00:00Z",
        )
        .method(),
        SYSTEM_EVENT_METHOD
    );
    assert_eq!(
        Command::new(
            "command-1",
            CommandName::Unknown("example/command".to_owned()),
        )
        .method(),
        COMMAND_METHOD
    );
}

#[test]
fn rpc_methods_are_short_and_unprefixed() {
    assert_eq!(SYSTEM_EVENT_METHOD, "system_event");
    assert_eq!(COMMAND_METHOD, "command");
    assert_eq!(ACP_METHOD, "acp");
}

#[test]
fn command_and_event_names_are_opaque_wire_strings() {
    let command_names = [
        CommandName::Unknown("runtime/configure".to_owned()),
        CommandName::Unknown("vendor/custom-command".to_owned()),
    ];
    for name in command_names {
        let wire_name = name.as_str();
        assert_eq!(serde_json::to_value(&name).unwrap(), json!(wire_name));
        assert_eq!(
            serde_json::from_value::<CommandName>(json!(wire_name)).unwrap(),
            name
        );
    }

    let event_names = [
        SystemEventName::Unknown("runtime/ready".to_owned()),
        SystemEventName::Unknown("agent/started".to_owned()),
        SystemEventName::Unknown("vendor/custom-event".to_owned()),
    ];
    for name in event_names {
        let wire_name = name.as_str();
        assert_eq!(serde_json::to_value(&name).unwrap(), json!(wire_name));
        assert_eq!(
            serde_json::from_value::<SystemEventName>(json!(wire_name)).unwrap(),
            name
        );
    }
}

#[test]
fn unknown_command_and_event_names_round_trip_losslessly() {
    let command = serde_json::from_value::<CommandName>(json!("vendor/custom-command")).unwrap();
    assert_eq!(
        command,
        CommandName::Unknown("vendor/custom-command".to_owned())
    );
    assert_eq!(
        serde_json::to_value(command).unwrap(),
        json!("vendor/custom-command")
    );

    let event = serde_json::from_value::<SystemEventName>(json!("vendor/custom-event")).unwrap();
    assert_eq!(
        event,
        SystemEventName::Unknown("vendor/custom-event".to_owned())
    );
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        json!("vendor/custom-event")
    );
}

#[test]
fn system_event_has_the_specified_wire_shape() {
    let event = SystemEvent::new(
        "019c-event",
        18,
        SystemEventName::Unknown("agent/stopped".to_owned()),
        "2026-07-17T18:42:10Z",
    )
    .agent("primary", "019c-agent-instance")
    .payload(json!({
        "reason": "process_exit",
        "exitCode": 1,
    }));

    let message = event.to_untyped_message().unwrap();

    assert_eq!(message.method(), SYSTEM_EVENT_METHOD);
    assert_eq!(
        message.params(),
        &json!({
            "eventId": "019c-event",
            "sequence": 18,
            "name": "agent/stopped",
            "occurredAt": "2026-07-17T18:42:10Z",
            "agentId": "primary",
            "agentInstanceId": "019c-agent-instance",
            "payload": {
                "reason": "process_exit",
                "exitCode": 1,
            },
        })
    );
}

#[test]
fn system_event_rejects_partially_scoped_targets() {
    let base = json!({
        "eventId": "event-1",
        "sequence": 1,
        "name": "agent/changed",
        "occurredAt": "2026-07-17T00:00:00Z",
    });

    for partial_target in [
        json!({ "agentId": "agent-1" }),
        json!({ "agentInstanceId": "instance-1" }),
    ] {
        let mut event = base.clone();
        event
            .as_object_mut()
            .unwrap()
            .extend(partial_target.as_object().unwrap().clone());

        assert!(
            serde_json::from_value::<SystemEvent>(event).is_err(),
            "partially scoped events must be rejected"
        );
    }
}

#[test]
fn command_and_results_have_the_specified_wire_shapes() {
    let command = Command::new(
        "019c-command",
        CommandName::Unknown("example/command".to_owned()),
    )
    .agent("primary")
    .agent_instance("019c-agent-instance")
    .payload(json!({ "reason": "configuration_changed" }));

    let message = command.to_untyped_message().unwrap();
    assert_eq!(message.method(), COMMAND_METHOD);
    assert_eq!(
        message.params(),
        &json!({
            "commandId": "019c-command",
            "name": "example/command",
            "agentId": "primary",
            "agentInstanceId": "019c-agent-instance",
            "payload": {
                "reason": "configuration_changed",
            },
        })
    );

    assert_eq!(
        CommandResult::completed_with(json!({ "handled": true }))
            .into_json(COMMAND_METHOD)
            .unwrap(),
        json!({
            "status": "completed",
            "value": { "handled": true },
        })
    );
    assert!(
        serde_json::from_value::<CommandResult>(json!({ "status": "accepted" })).is_err(),
        "the removed acceptance result must not remain wire-compatible"
    );
}

#[test]
fn acp_message_contains_an_acp_raw_jsonrpc_message() {
    let nested = RawJsonRpcMessage::request(
        "initialize".to_owned(),
        json!({ "protocolVersion": 1 }),
        RequestId::Number(1),
    )
    .unwrap();
    let delivery = AcpMessage::new("019c-acp-message", "primary", "019c-agent-instance", nested);

    let message = delivery.to_untyped_message().unwrap();

    assert_eq!(message.method(), ACP_METHOD);
    assert_eq!(
        message.params(),
        &json!({
            "messageId": "019c-acp-message",
            "agentId": "primary",
            "agentInstanceId": "019c-agent-instance",
            "message": {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                },
            },
        })
    );

    let parsed = AcpMessage::parse_message(message.method(), message.params()).unwrap();
    assert_eq!(
        serde_json::to_value(parsed.message).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
            },
        })
    );
}
