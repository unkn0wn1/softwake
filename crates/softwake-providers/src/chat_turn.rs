//! One chat/completions turn that may return assistant text or `tool_calls`.
//!
//! Softwake stays on the legacy `/chat/completions` endpoint. Tools use the
//! OpenAI-compatible nested `function` shape xAI accepts. See ADR-0025.

use serde_json::{Value, json};

use crate::chat::{CHAT_MAX_TOKENS, ChatError, ChatMessage, PreparedChat};
use crate::transport::{Transport, TransportError};

/// One function tool call from the assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantToolCall {
    /// Provider-assigned call id (required on tool result messages).
    pub id: String,
    /// Registered Softwake tool name.
    pub name: String,
    /// Raw JSON arguments object (string form from the wire).
    pub arguments: String,
}

/// Result of one completion POST when tools may be present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatTurn {
    /// Final assistant text (trimmed, non-empty).
    Message(String),
    /// Model requested one or more tool calls. `content` may be empty.
    ToolCalls {
        /// Optional assistant prose beside the calls.
        content: Option<String>,
        /// Calls to execute before the next POST.
        calls: Vec<AssistantToolCall>,
    },
}

/// Role on a non-system wire message for a tool-capable turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireRole {
    /// User / operator.
    User,
    /// Assistant (text and/or `tool_calls`).
    Assistant,
    /// Tool result for a prior `tool_call_id`.
    Tool,
}

/// One user, assistant, or tool message on the wire (system is separate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMessage {
    /// Message role.
    pub role: WireRole,
    /// Text content. Required for user/tool; optional for assistant with tools.
    pub content: Option<String>,
    /// Assistant `tool_calls` when the model requested them.
    pub tool_calls: Option<Vec<AssistantToolCall>>,
    /// Tool result id when `role` is [`WireRole::Tool`].
    pub tool_call_id: Option<String>,
}

impl WireMessage {
    /// User text turn.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: WireRole::User,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Assistant text turn (no tools).
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: WireRole::Assistant,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Assistant turn that requested tool calls.
    #[must_use]
    pub fn assistant_tools(content: Option<String>, calls: Vec<AssistantToolCall>) -> Self {
        Self {
            role: WireRole::Assistant,
            content,
            tool_calls: Some(calls),
            tool_call_id: None,
        }
    }

    /// Tool result for `tool_call_id`.
    #[must_use]
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: WireRole::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Convert session-style chat messages into wire messages (text only).
#[must_use]
pub fn wire_from_chat_messages(messages: &[ChatMessage]) -> Vec<WireMessage> {
    messages
        .iter()
        .map(|message| match message.role {
            crate::chat::ChatRole::User => WireMessage::user(message.content.clone()),
            crate::chat::ChatRole::Assistant => WireMessage::assistant(message.content.clone()),
        })
        .collect()
}

/// POST one chat completion that may include `tools`.
///
/// When `tools` is empty the `tools` field is omitted (same as [`crate::complete_chat`]).
///
/// # Errors
///
/// [`ChatError`] on transport/status/parse failures. Empty final text is
/// [`ChatError::Empty`]. A `tool_calls` turn with an empty `calls` array is
/// [`ChatError::Unparseable`].
pub fn complete_chat_turn<T: Transport>(
    transport: &T,
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    messages: &[WireMessage],
    tools: &[Value],
) -> Result<ChatTurn, ChatError> {
    let url = format!("{}/chat/completions", prepared.api_base);
    let mut wire = Vec::with_capacity(messages.len() + 1);
    wire.push(json!({"role": "system", "content": system}));
    for message in messages {
        wire.push(wire_message_json(message));
    }
    let mut body = json!({
        "model": prepared.model,
        "max_tokens": CHAT_MAX_TOKENS,
        "messages": wire,
    });
    crate::reasoning::insert_reasoning_effort(&mut body, &prepared.reasoning_effort);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    parse_chat_turn(transport.post_json_bearer(&url, bearer, &body.to_string()))
}

/// Stream a chat turn (`stream: true`) with optional `tools` (ADR-0048 / stream-feel).
///
/// Content deltas invoke `on_delta` with accumulated assistant text. Tool-call
/// fragments are merged by `index`. Return `false` from `on_delta` to cancel.
///
/// # Errors
///
/// [`ChatError`] on transport failure, empty Message, or empty `tool_calls`.
pub fn complete_chat_turn_stream<T: Transport>(
    transport: &T,
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    messages: &[WireMessage],
    tools: &[Value],
    mut on_delta: impl FnMut(&str) -> bool,
) -> Result<ChatTurn, ChatError> {
    let url = format!("{}/chat/completions", prepared.api_base);
    let mut wire = Vec::with_capacity(messages.len() + 1);
    wire.push(json!({"role": "system", "content": system}));
    for message in messages {
        wire.push(wire_message_json(message));
    }
    let mut body = json!({
        "model": prepared.model,
        "max_tokens": CHAT_MAX_TOKENS,
        "messages": wire,
        "stream": true,
    });
    crate::reasoning::insert_reasoning_effort(&mut body, &prepared.reasoning_effort);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    let mut assembled = String::new();
    let mut call_bufs: Vec<StreamToolCallBuf> = Vec::new();
    let result = transport.post_json_bearer_stream(&url, bearer, &body.to_string(), &mut |event| {
        apply_tool_call_deltas(event, &mut call_bufs);
        if let Some(piece) = crate::chat::delta_content_from_sse_data(event) {
            assembled.push_str(&piece);
            if !on_delta(&assembled) {
                return false;
            }
        }
        true
    });
    match result {
        Ok(()) => finish_streamed_turn(&assembled, call_bufs),
        Err(TransportError::Failed { message })
            if message.contains("401") || message.contains("403") =>
        {
            Err(ChatError::Rejected)
        }
        Err(TransportError::Failed { .. } | TransportError::NoRoute { .. }) => {
            Err(ChatError::Unreachable)
        }
    }
}

/// Stream a **text-only** chat turn (`stream: true`, no `tools`).
///
/// Wrapper over [`complete_chat_turn_stream`] for tool-free asks / finalize.
///
/// # Errors
///
/// [`ChatError`] on transport failure or empty final text.
pub fn complete_chat_turn_text_stream<T: Transport>(
    transport: &T,
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    messages: &[WireMessage],
    on_delta: impl FnMut(&str) -> bool,
) -> Result<String, ChatError> {
    match complete_chat_turn_stream(transport, prepared, bearer, system, messages, &[], on_delta)? {
        ChatTurn::Message(text) => Ok(text),
        ChatTurn::ToolCalls { content, .. } => {
            let trimmed = content.unwrap_or_default();
            let trimmed = trimmed.trim();
            if trimmed.is_empty() {
                Err(ChatError::Empty)
            } else {
                Ok(trimmed.to_owned())
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
struct StreamToolCallBuf {
    id: String,
    name: String,
    arguments: String,
}

fn apply_tool_call_deltas(data: &str, bufs: &mut Vec<StreamToolCallBuf>) {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return;
    };
    let Some(calls) = value
        .pointer("/choices/0/delta/tool_calls")
        .and_then(Value::as_array)
    else {
        return;
    };
    for call in calls {
        let index = call
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(0);
        while bufs.len() <= index {
            bufs.push(StreamToolCallBuf::default());
        }
        let slot = &mut bufs[index];
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                id.clone_into(&mut slot.id);
            }
        }
        if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
            if !name.is_empty() {
                slot.name.push_str(name);
            }
        }
        if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
            slot.arguments.push_str(args);
        }
    }
}

fn finish_streamed_turn(
    assembled: &str,
    call_bufs: Vec<StreamToolCallBuf>,
) -> Result<ChatTurn, ChatError> {
    let calls: Vec<AssistantToolCall> = call_bufs
        .into_iter()
        .filter(|buf| !buf.id.is_empty() && !buf.name.is_empty())
        .map(|buf| AssistantToolCall {
            id: buf.id,
            name: buf.name,
            arguments: if buf.arguments.is_empty() {
                "{}".to_owned()
            } else {
                buf.arguments
            },
        })
        .collect();
    if !calls.is_empty() {
        let content = {
            let trimmed = assembled.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        };
        return Ok(ChatTurn::ToolCalls { content, calls });
    }
    let trimmed = assembled.trim();
    if trimmed.is_empty() {
        return Err(ChatError::Empty);
    }
    Ok(ChatTurn::Message(trimmed.to_owned()))
}

fn wire_message_json(message: &WireMessage) -> Value {
    match message.role {
        WireRole::User => json!({
            "role": "user",
            "content": message.content.clone().unwrap_or_default(),
        }),
        WireRole::Tool => json!({
            "role": "tool",
            "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
            "content": message.content.clone().unwrap_or_default(),
        }),
        WireRole::Assistant => {
            let mut obj = json!({"role": "assistant"});
            if let Some(calls) = &message.tool_calls {
                obj["tool_calls"] = Value::Array(
                    calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments,
                                }
                            })
                        })
                        .collect(),
                );
                match &message.content {
                    Some(text) if !text.is_empty() => {
                        obj["content"] = Value::String(text.clone());
                    }
                    _ => {
                        obj["content"] = Value::Null;
                    }
                }
            } else {
                obj["content"] = Value::String(message.content.clone().unwrap_or_default());
            }
            obj
        }
    }
}

fn parse_chat_turn(
    response: Result<crate::transport::HttpResponse, TransportError>,
) -> Result<ChatTurn, ChatError> {
    let response = response.map_err(|error| match error {
        TransportError::Failed { .. } | TransportError::NoRoute { .. } => ChatError::Unreachable,
    })?;
    match response.status {
        401 | 403 => return Err(ChatError::Rejected),
        status if !(200..300).contains(&status) => return Err(ChatError::Failed { status }),
        _ => {}
    }
    let body: Value = serde_json::from_str(&response.body).map_err(|_| ChatError::Unparseable)?;
    let message = body
        .pointer("/choices/0/message")
        .ok_or(ChatError::Unparseable)?;
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        if calls.is_empty() {
            return Err(ChatError::Unparseable);
        }
        let mut parsed = Vec::with_capacity(calls.len());
        for call in calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .ok_or(ChatError::Unparseable)?;
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .ok_or(ChatError::Unparseable)?;
            let arguments = match call.pointer("/function/arguments") {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => return Err(ChatError::Unparseable),
            };
            parsed.push(AssistantToolCall {
                id: id.to_owned(),
                name: name.to_owned(),
                arguments,
            });
        }
        let content = match message.get("content") {
            Some(Value::String(text)) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            }
            Some(Value::Null) | None => None,
            Some(_) => return Err(ChatError::Unparseable),
        };
        return Ok(ChatTurn::ToolCalls {
            content,
            calls: parsed,
        });
    }
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or(ChatError::Unparseable)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(ChatError::Empty);
    }
    Ok(ChatTurn::Message(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use serde_json::{Value, json};

    use super::{
        AssistantToolCall, ChatTurn, WireMessage, complete_chat_turn, complete_chat_turn_stream,
        wire_from_chat_messages,
    };
    use crate::chat::{ChatMessage, PreparedChat};
    use crate::ids::ProviderId;
    use crate::registry::ProviderFamily;
    use crate::transport::{HttpBytes, HttpResponse, MultipartField, Transport, TransportError};

    struct QueueTransport {
        posts: RefCell<Vec<(String, String)>>,
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
            url: &str,
            _bearer: &str,
            body: &str,
        ) -> Result<HttpResponse, TransportError> {
            self.posts
                .borrow_mut()
                .push((url.to_owned(), body.to_owned()));
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| TransportError::NoRoute {
                    method: "POST".to_owned(),
                    url: url.to_owned(),
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
            reasoning_effort: String::new(),
        }
    }

    #[test]
    fn complete_chat_turn_omits_tools_when_empty_and_returns_text() {
        let transport = QueueTransport::new(vec![HttpResponse {
            status: 200,
            body: json!({
                "choices": [{"message": {"role": "assistant", "content": "pong"}}]
            })
            .to_string(),
        }]);
        let turn = complete_chat_turn(
            &transport,
            &prepared(),
            "tok",
            "sys",
            &[WireMessage::user("hi")],
            &[],
        )
        .expect("text");
        assert_eq!(turn, ChatTurn::Message("pong".to_owned()));
        let body: Value = serde_json::from_str(&transport.posts.borrow()[0].1).expect("json");
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn complete_chat_turn_posts_tools_and_parses_tool_calls() {
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
                                "arguments": "{\"command\":\"free -h\"}"
                            }
                        }]
                    }
                }]
            })
            .to_string(),
        }]);
        let turn = complete_chat_turn(
            &transport,
            &prepared(),
            "tok",
            "sys",
            &[WireMessage::user("check swap")],
            &tools,
        )
        .expect("tools");
        match turn {
            ChatTurn::ToolCalls { content, calls } => {
                assert!(content.is_none());
                assert_eq!(
                    calls,
                    vec![AssistantToolCall {
                        id: "call_1".to_owned(),
                        name: "shell".to_owned(),
                        arguments: "{\"command\":\"free -h\"}".to_owned(),
                    }]
                );
            }
            ChatTurn::Message(_) => panic!("expected tool calls"),
        }
        let body: Value = serde_json::from_str(&transport.posts.borrow()[0].1).expect("json");
        assert_eq!(body["tools"], Value::Array(tools));
    }

    #[test]
    fn wire_from_chat_messages_preserves_roles() {
        let wire = wire_from_chat_messages(&[ChatMessage::user("u"), ChatMessage::assistant("a")]);
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0], WireMessage::user("u"));
        assert_eq!(wire[1], WireMessage::assistant("a"));
    }
    #[test]
    fn complete_chat_turn_stream_assembles_text_with_tools_advertised() {
        let transport = crate::MockTransport::default().with_stream_events([
            r#"{"choices":[{"delta":{"content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo"}}]}"#,
        ]);
        let tools = vec![json!({
            "type": "function",
            "function": {"name": "echo", "description": "Echo", "parameters": {"type": "object"}}
        })];
        let mut seen = Vec::new();
        let turn = complete_chat_turn_stream(
            &transport,
            &PreparedChat {
                provider: ProviderId::XaiKey,
                family: ProviderFamily::Xai,
                api_base: "https://api.x.ai/v1".into(),
                model: "grok-test".into(),
                reasoning_effort: String::new(),
            },
            "sk-test",
            "sys",
            &[WireMessage::user("hi")],
            &tools,
            |partial| {
                seen.push(partial.to_owned());
                true
            },
        )
        .expect("stream");
        assert_eq!(turn, ChatTurn::Message("Hello".to_owned()));
        assert_eq!(seen, vec!["Hel".to_owned(), "Hello".to_owned()]);
    }

    #[test]
    fn complete_chat_turn_stream_merges_tool_call_deltas() {
        let transport = crate::MockTransport::default().with_stream_events([
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"echo","arguments":"{\"t\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"hi\"}"}}]}}]}"#,
        ]);
        let turn = complete_chat_turn_stream(
            &transport,
            &PreparedChat {
                provider: ProviderId::XaiKey,
                family: ProviderFamily::Xai,
                api_base: "https://api.x.ai/v1".into(),
                model: "grok-test".into(),
                reasoning_effort: String::new(),
            },
            "sk-test",
            "sys",
            &[WireMessage::user("hi")],
            &[],
            |_| true,
        )
        .expect("stream tools");
        match turn {
            ChatTurn::ToolCalls { content, calls } => {
                assert!(content.is_none());
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "echo");
                assert_eq!(calls[0].arguments, r#"{"t":"hi"}"#);
            }
            ChatTurn::Message(_) => panic!("expected tool calls"),
        }
    }
}
