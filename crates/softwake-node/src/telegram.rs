//! Telegram sticky long-poll on the companion (ADR-0042).
//!
//! Polls only when desired owner is `companion` (laptop not present + mirrored
//! bot token). Bounded xAI oneshot when a key is available; else stub reply.
//! TTS is skipped on the node (no audio stack).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::state::{NodeState, OutboxItem, TelegramOwner};

const API_ROOT: &str = "https://api.telegram.org";
const POLL_TIMEOUT_SECS: u64 = 25;
const DEFAULT_MODEL: &str = "grok-4-fast-non-reasoning";
static OFFSET: AtomicI64 = AtomicI64::new(0);

/// Spawn the companion Telegram supervisor thread.
pub fn spawn(state: Arc<NodeState>) {
    let _ = thread::Builder::new()
        .name("softwake-node-telegram".into())
        .spawn(move || {
            loop {
                let now = NodeState::now_ms();
                let (owner, _presence, has_token) = state.telegram_ownership_snapshot(now);
                if owner != TelegramOwner::Companion || !has_token {
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
                let Some(token) = state.telegram_bot_token() else {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                };
                // Re-check ownership after long-poll setup intent.
                if state.desired_telegram_owner(NodeState::now_ms()) != TelegramOwner::Companion {
                    thread::sleep(Duration::from_millis(200));
                    continue;
                }
                match poll_once(&state, &token) {
                    Ok(0) => thread::sleep(Duration::from_millis(200)),
                    Ok(_) => {}
                    Err(_) => thread::sleep(Duration::from_secs(2)),
                }
            }
        });
}

fn poll_once(state: &NodeState, token: &str) -> Result<usize, String> {
    // Bail if laptop returned present mid-wait setup.
    if state.desired_telegram_owner(NodeState::now_ms()) != TelegramOwner::Companion {
        return Ok(0);
    }
    let offset = OFFSET.load(Ordering::Relaxed);
    let updates = get_updates(token, offset)?;
    // If ownership flipped during the long-poll, do not process (laptop will).
    if state.desired_telegram_owner(NodeState::now_ms()) != TelegramOwner::Companion {
        return Ok(0);
    }
    let mut n = 0;
    for update in updates {
        if update.update_id >= offset {
            OFFSET.store(update.update_id + 1, Ordering::Relaxed);
        }
        if let Some(message) = update.message {
            handle_inbound(state, token, message);
            n += 1;
        }
    }
    Ok(n)
}

#[derive(Debug, Deserialize)]
struct TgUpdates {
    ok: bool,
    #[serde(default)]
    result: Vec<TgUpdate>,
}

#[derive(Debug, Deserialize)]
struct TgUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TgMessage>,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    #[serde(default)]
    text: Option<String>,
    chat: TgChat,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
}

fn get_updates(token: &str, offset: i64) -> Result<Vec<TgUpdate>, String> {
    let url = format!(
        "{API_ROOT}/bot{token}/getUpdates?offset={offset}&timeout={POLL_TIMEOUT_SECS}&allowed_updates=%5B%22message%22%5D"
    );
    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(POLL_TIMEOUT_SECS + 10))
        .timeout_connect(Duration::from_secs(15))
        .build();
    let response = agent.get(&url).call().map_err(|e| e.to_string())?;
    let raw = response.into_string().map_err(|e| e.to_string())?;
    let body: TgUpdates = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if !body.ok {
        return Err("telegram getUpdates not ok".into());
    }
    Ok(body.result)
}

fn handle_inbound(state: &NodeState, token: &str, message: TgMessage) {
    let Some(text) = message
        .text
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    else {
        return;
    };
    let chat_id = message.chat.id.to_string();
    let profile_id = state.find_profile_for_chat(&chat_id);
    state.bind_chat_id(&profile_id, &chat_id);
    // Mirrored messengers drive profile routing/bind; inbound always gets a reply
    // while companion owns the poll (Default / Receive-all / Voice flags apply to
    // laptop-side fan-out; TTS is skipped on the node).
    let _messengers = state.load_messengers(&profile_id);
    let reply = companion_ask(state, &text);
    let _ = send_message(token, &chat_id, &reply);
    // Voice/TTS skipped on node (ADR-0042).
    let summary = format!(
        "Telegram (companion): user said «{}» → «{}»",
        truncate(&text, 80),
        truncate(&reply, 120)
    );
    state.push_outbox(OutboxItem {
        id: format!("ob-tg-{}", NodeState::now_ms()),
        profile_id,
        kind: "telegram_inbound".into(),
        schedule_id: None,
        ts_ms: NodeState::now_ms(),
        summary,
        lease_id: None,
    });
}

fn companion_ask(state: &NodeState, text: &str) -> String {
    let Some(key) = state.xai_api_key() else {
        return "Softwake companion here (laptop away). Full agent replies when the laptop is back."
            .to_owned();
    };
    match xai_oneshot(&key, text) {
        Ok(reply) if !reply.trim().is_empty() => reply,
        Ok(_) | Err(_) => {
            "Softwake companion here (laptop away). I could not reach the model just now."
                .to_owned()
        }
    }
}

fn xai_oneshot(api_key: &str, text: &str) -> Result<String, String> {
    let model = std::env::var("SOFTWAKE_NODE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "You are Softwake's companion node answering Telegram while the laptop Softwake is away. Be concise. No tools."
            },
            {"role": "user", "content": text}
        ],
        "max_tokens": 400,
        "temperature": 0.4
    });
    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(45))
        .timeout_connect(Duration::from_secs(15))
        .build();
    let response = agent
        .post("https://api.x.ai/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let raw = response.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_owned();
    if content.is_empty() {
        return Err("empty model content".into());
    }
    Ok(content)
}

fn send_message(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    let url = format!("{API_ROOT}/bot{token}/sendMessage");
    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });
    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(20))
        .timeout_connect(Duration::from_secs(15))
        .build();
    let _ = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate("hi", 10), "hi");
        assert!(truncate("abcdefghij", 5).ends_with('…'));
    }
}
