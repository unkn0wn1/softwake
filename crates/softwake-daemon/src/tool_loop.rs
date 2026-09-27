//! Multi-turn chat tool loop: assistant `tool_calls` → Hands → tool results → continue.
//!
//! Caps rounds so a model cannot spin forever. Never invents command output.
//! See ADR-0025.
//!
//! The loop body runs under `live-http` and in unit tests. Default CI builds keep
//! the types so ask paths can compile without advertising a second HTTP stack.

use serde_json::Value;
use softwake_providers::ChatMessage;
use softwake_providers::{
    AssistantToolCall, ChatTurn, PreparedChat, Transport, WireMessage, complete_chat_turn,
    wire_from_chat_messages,
};
use softwake_tools::tool_args_from_json;

use crate::dispatch::{PendingToolCall, RequestOutcome};

/// Max assistant→tools→continue rounds per ask (each round is one HTTP POST).
#[cfg_attr(not(any(test, feature = "live-http")), allow(dead_code))]
pub(crate) const MAX_TOOL_ROUNDS: usize = 6;

/// Result of asking Hands to run one tool from an API `tool_call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolInvokeResult {
    /// Tool ran; content is real detail for the tool role message.
    Ran(String),
    /// Ask-mode staged a pending confirmation. Stop the loop.
    Pending(PendingToolCall),
    /// Refusal or failure text to feed back as the tool result (continue).
    Failed(String),
}

/// Successful tool-loop finish.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(any(test, feature = "live-http")), allow(dead_code))]
pub(crate) enum ToolLoopOk {
    /// Final assistant text for the session.
    Message(String),
    /// Loop stopped because a tool is waiting for HUD Approve.
    Pending {
        /// Operator-facing sentence stored as the assistant reply.
        message: String,
        /// Pending record already held by Hands.
        pending: PendingToolCall,
    },
}

/// Run chat turns until final text, pending confirm, or caps.
///
/// `messages` are the session turns including the new user line (text only).
/// `tools` is the advertised list (may be empty). `invoke` maps one call onto Hands.
///
/// # Errors
///
/// Provider display sentences, or a round-cap message. Never includes the bearer.
#[cfg_attr(not(any(test, feature = "live-http")), allow(dead_code))]
pub(crate) fn run_tool_loop<T: Transport>(
    transport: &T,
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    messages: &[ChatMessage],
    tools: &[Value],
    mut invoke: impl FnMut(&str, &[String]) -> ToolInvokeResult,
) -> Result<ToolLoopOk, String> {
    let mut wire = wire_from_chat_messages(messages);
    for round in 0..MAX_TOOL_ROUNDS {
        let _ = round;
        let turn = complete_chat_turn(transport, prepared, bearer, system, &wire, tools)
            .map_err(|error| error.to_string())?;
        match turn {
            ChatTurn::Message(text) => return Ok(ToolLoopOk::Message(text)),
            ChatTurn::ToolCalls { content, calls } => {
                wire.push(WireMessage::assistant_tools(content, calls.clone()));
                let mut pending_stop: Option<PendingToolCall> = None;
                for call in &calls {
                    if let Some(pending) = &pending_stop {
                        wire.push(WireMessage::tool(
                            call.id.clone(),
                            format!(
                                "skipped: confirmation {} for {} is already pending",
                                pending.pending_id, pending.name
                            ),
                        ));
                        continue;
                    }
                    let result = dispatch_one(&mut invoke, call);
                    match result {
                        ToolInvokeResult::Ran(detail) => {
                            wire.push(WireMessage::tool(call.id.clone(), detail));
                        }
                        ToolInvokeResult::Failed(message) => {
                            wire.push(WireMessage::tool(call.id.clone(), message));
                        }
                        ToolInvokeResult::Pending(pending) => {
                            wire.push(WireMessage::tool(
                                call.id.clone(),
                                format!(
                                    "pending confirmation {}: waiting for operator Approve",
                                    pending.pending_id
                                ),
                            ));
                            pending_stop = Some(pending);
                        }
                    }
                }
                if let Some(pending) = pending_stop {
                    let message = format!(
                        "pending confirmation {} for {}",
                        pending.pending_id, pending.name
                    );
                    return Ok(ToolLoopOk::Pending { message, pending });
                }
            }
        }
    }
    Err(format!(
        "tool loop reached the {MAX_TOOL_ROUNDS}-round cap without a final reply"
    ))
}

#[cfg_attr(not(any(test, feature = "live-http")), allow(dead_code))]
fn dispatch_one(
    invoke: &mut impl FnMut(&str, &[String]) -> ToolInvokeResult,
    call: &AssistantToolCall,
) -> ToolInvokeResult {
    match tool_args_from_json(&call.name, &call.arguments) {
        Ok(args) => invoke(&call.name, &args),
        Err(_message) if softwake_tools::is_mcp_tool_name(&call.name) => {
            let args = vec![call.arguments.clone()];
            invoke(&call.name, &args)
        }
        Err(message) => ToolInvokeResult::Failed(message),
    }
}

/// Map a Hands [`RequestOutcome`] into a loop invoke result.
#[must_use]
pub(crate) fn invoke_from_request(
    step: Result<RequestOutcome, crate::dispatch::DispatchError>,
) -> ToolInvokeResult {
    match step {
        Ok(RequestOutcome::Ran(ran)) => ToolInvokeResult::Ran(ran.detail),
        Ok(RequestOutcome::Pending(pending)) => ToolInvokeResult::Pending(pending),
        Err(error) => ToolInvokeResult::Failed(error.to_string()),
    }
}

/// Build chat messages from session role/content pairs (test helper shape).
#[cfg(test)]
#[must_use]
pub(crate) fn chat_user(text: &str) -> ChatMessage {
    use softwake_providers::ChatRole;
    ChatMessage {
        role: ChatRole::User,
        content: text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use serde_json::json;
    use softwake_providers::{
        HttpBytes, HttpResponse, MultipartField, PreparedChat, ProviderFamily, ProviderId,
        Transport, TransportError,
    };

    use super::{ToolInvokeResult, ToolLoopOk, chat_user, run_tool_loop};
    use crate::dispatch::PendingToolCall;

    struct QueueTransport {
        posts: RefCell<Vec<String>>,
        responses: RefCell<VecDeque<HttpResponse>>,
    }

    impl QueueTransport {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                posts: RefCell::new(Vec::new()),
                responses: RefCell::new(VecDeque::from(responses)),
            }
        }
    }

    impl Transport for QueueTransport {
        fn post_form(&self, url: &str, _body: &str) -> Result<HttpResponse, TransportError> {
            Err(TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
        }

        fn get_bearer(&self, url: &str, _bearer: &str) -> Result<HttpResponse, TransportError> {
            Err(TransportError::NoRoute {
                method: "GET".to_owned(),
                url: url.to_owned(),
            })
        }

        fn post_json_bearer(
            &self,
            _url: &str,
            _bearer: &str,
            body: &str,
        ) -> Result<HttpResponse, TransportError> {
            self.posts.borrow_mut().push(body.to_owned());
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| TransportError::Failed {
                    message: "no more scripted responses".to_owned(),
                })
        }

        fn post_multipart_bearer(
            &self,
            url: &str,
            _bearer: &str,
            _fields: &[MultipartField],
            _file_name: &str,
            _file_bytes: &[u8],
            _file_content_type: &str,
        ) -> Result<HttpResponse, TransportError> {
            Err(TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
        }

        fn post_json_bearer_bytes(
            &self,
            url: &str,
            _bearer: &str,
            _body: &str,
        ) -> Result<HttpBytes, TransportError> {
            Err(TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
        }
    }

    fn prepared() -> PreparedChat {
        PreparedChat {
            provider: ProviderId::XaiKey,
            family: ProviderFamily::Xai,
            api_base: "https://api.x.ai/v1".to_owned(),
            model: "grok-4.5".to_owned(),
        }
    }

    #[test]
    fn loop_runs_shell_then_returns_final_text_with_real_detail() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run shell",
                "parameters": {
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"]
                }
            }
        })];
        let transport = QueueTransport::new(vec![
            HttpResponse {
                status: 200,
                body: json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "shell",
                                    "arguments": "{\"command\":\"echo real-out\"}"
                                }
                            }]
                        }
                    }]
                })
                .to_string(),
            },
            HttpResponse {
                status: 200,
                body: json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "Swap looks fine: real-out"
                        }
                    }]
                })
                .to_string(),
            },
        ]);
        let invokes = RefCell::new(Vec::new());
        let ok = run_tool_loop(
            &transport,
            &prepared(),
            "tok",
            "sys",
            &[chat_user("please check swap")],
            &tools,
            |name, args| {
                invokes.borrow_mut().push((name.to_owned(), args.to_vec()));
                assert_eq!(name, "shell");
                assert_eq!(args, &["echo real-out".to_owned()]);
                ToolInvokeResult::Ran("stdout:\nreal-out\n".to_owned())
            },
        )
        .expect("loop");
        assert_eq!(
            ok,
            ToolLoopOk::Message("Swap looks fine: real-out".to_owned())
        );
        assert_eq!(transport.posts.borrow().len(), 2);
        let second: serde_json::Value =
            serde_json::from_str(&transport.posts.borrow()[1]).expect("json");
        let tool_msg = second["messages"]
            .as_array()
            .expect("msgs")
            .iter()
            .find(|m| m["role"] == "tool")
            .expect("tool");
        assert_eq!(tool_msg["content"], "stdout:\nreal-out\n");
        assert_eq!(invokes.borrow().len(), 1);
    }

    #[test]
    fn loop_stops_on_pending_without_second_post() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run shell",
                "parameters": {
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"]
                }
            }
        })];
        let transport = QueueTransport::new(vec![HttpResponse {
            status: 200,
            body: json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "shell",
                                "arguments": "{\"command\":\"rm -rf /\"}"
                            }
                        }]
                    }
                }]
            })
            .to_string(),
        }]);
        let ok = run_tool_loop(
            &transport,
            &prepared(),
            "tok",
            "sys",
            &[chat_user("wipe disk")],
            &tools,
            |name, args| {
                assert_eq!(name, "shell");
                ToolInvokeResult::Pending(PendingToolCall {
                    pending_id: "1".to_owned(),
                    name: name.to_owned(),
                    args: args.to_vec(),
                    description: "rm -rf /".to_owned(),
                })
            },
        )
        .expect("pending");
        match ok {
            ToolLoopOk::Pending { message, pending } => {
                assert!(message.contains("pending confirmation 1"));
                assert_eq!(pending.name, "shell");
            }
            ToolLoopOk::Message(_) => panic!("expected pending"),
        }
        assert_eq!(transport.posts.borrow().len(), 1);
    }
}
