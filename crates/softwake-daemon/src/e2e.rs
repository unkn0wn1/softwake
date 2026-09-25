//! Socket scenarios for serve and ctl. No window is opened.

use std::fs;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use softwake_ipc::{
    Client, ClientMessage, Command, Event, PROTOCOL_VERSION, ServerMessage, VoiceState,
    read_message, write_message,
};

use crate::ctl;
use crate::serve;
use crate::soul::{TestSoulDir, reload_message};

struct TempSocket {
    dir: PathBuf,
    path: PathBuf,
}

impl TempSocket {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("sw-daemon-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        Self {
            path: dir.join("s"),
            dir,
        }
    }
}

impl Drop for TempSocket {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn ctl_hibernate_then_resume_lands_in_sleep() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");

    let status = ctl::call(temp.path(), Command::GetStatus).expect("status");
    assert_eq!(status.state, VoiceState::Sleep);
    assert!(status.capture_running);
    assert!(!status.soul_reload_pending);
    assert!(status.soul.as_ref().is_some_and(|report| report.ok));

    let hibernated = ctl::call(temp.path(), Command::Hibernate).expect("hibernate");
    assert_eq!(hibernated.state, VoiceState::Hibernate);
    assert!(!hibernated.capture_running);
    assert_eq!(hibernated.detail.as_deref(), Some("sleep -> hibernate"));

    let after = ctl::call(temp.path(), Command::GetStatus).expect("status");
    assert_eq!(after.state, VoiceState::Hibernate);
    assert!(!after.capture_running);

    let resumed = ctl::call(temp.path(), Command::WakeFromUi).expect("resume");
    assert_eq!(resumed.state, VoiceState::Sleep);
    assert!(resumed.capture_running);
    assert_ne!(resumed.state, VoiceState::Awake);

    let slept = ctl::call(temp.path(), Command::Sleep).expect_err("sleep");
    assert!(slept.to_string().contains("already asleep"));
    let still = ctl::call(temp.path(), Command::GetStatus).expect("status");
    assert_eq!(still.state, VoiceState::Sleep);
    assert!(still.capture_running);

    drop(server);
}

#[test]
fn reload_soul_rereads_and_a_second_client_sees_state_changed() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");

    let mut watcher = Client::connect(temp.path()).expect("watcher");
    watcher
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let reloaded = ctl::call(temp.path(), Command::ReloadSoul).expect("reload");
    assert!(reloaded.soul_reload_pending);
    assert!(reloaded.soul.as_ref().is_some_and(|report| report.ok));
    let message = reload_message(true, None);
    assert_eq!(reloaded.message.as_deref(), Some(message.as_str()));
    assert_eq!(
        ctl::format_status(&reloaded),
        format!("state: sleep\ncapture: running\nsoul: ok\nsoul reload: pending\n{message}\n")
    );

    let hibernated = ctl::call(temp.path(), Command::Hibernate).expect("hibernate");
    assert_eq!(hibernated.state, VoiceState::Hibernate);
    assert!(hibernated.soul_reload_pending);
    assert!(hibernated.message.is_none());

    let event = watcher.read().expect("event");
    match event {
        ServerMessage::Event {
            body:
                Event::StateChanged {
                    state,
                    previous,
                    capture_running,
                    ..
                },
        } => {
            assert_eq!(state, VoiceState::Hibernate);
            assert_eq!(previous, VoiceState::Sleep);
            assert!(!capture_running);
        }
        other => panic!("expected state_changed, got {other:?}"),
    }

    drop(watcher);
    drop(server);
}

#[test]
fn a_live_socket_is_kept_and_a_bad_hello_does_not_stop_serve() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");
    let error = serve::spawn(temp.path.clone(), soul.soul_dir()).expect_err("second");
    assert!(error.to_string().contains("is listening"), "{error}");

    let stream = UnixStream::connect(temp.path()).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    write_message(
        &mut writer,
        &ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
        },
    )
    .expect("hello");
    let reply: ServerMessage = read_message(&mut reader).expect("rejected");
    assert!(matches!(
        reply,
        ServerMessage::HelloRejected {
            expected: PROTOCOL_VERSION,
            ..
        }
    ));

    let status = ctl::call(temp.path(), Command::GetStatus).expect("still up");
    assert_eq!(status.state, VoiceState::Sleep);
    drop(server);

    let restarted = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("restart");
    drop(restarted);
}

#[test]
fn status_reports_a_missing_soul_and_hibernate_still_works() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::empty();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");

    let status = ctl::call(temp.path(), Command::GetStatus).expect("status");
    assert_eq!(status.state, VoiceState::Sleep);
    assert!(status.capture_running);
    let text = ctl::format_status(&status);
    assert!(text.contains("soul: missing"), "{text}");
    assert!(text.contains("missing soul.md"), "{text}");
    let report = status.soul.expect("soul report");
    assert!(!report.ok);
    let reason = report.reason.expect("reason");
    assert!(reason.contains("missing soul.md"), "{reason}");

    let hibernated = ctl::call(temp.path(), Command::Hibernate).expect("hibernate");
    assert_eq!(hibernated.state, VoiceState::Hibernate);
    assert!(!hibernated.capture_running);
    assert!(hibernated.soul.as_ref().is_some_and(|report| !report.ok));

    let resumed = ctl::call(temp.path(), Command::WakeFromUi).expect("resume");
    assert_eq!(resumed.state, VoiceState::Sleep);
    assert!(resumed.capture_running);

    soul.write("now soul\n", "now user\n");
    let reloaded = ctl::call(temp.path(), Command::ReloadSoul).expect("reload");
    assert!(reloaded.soul_reload_pending);
    assert!(reloaded.soul.as_ref().is_some_and(|report| report.ok));
    assert_eq!(reloaded.state, VoiceState::Sleep);

    drop(server);
}

impl TempSocket {
    fn path(&self) -> &Path {
        &self.path
    }
}

#[test]
fn ctl_tool_is_refused_until_awake_then_echo_is_deterministic() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");

    let asleep = ctl::call_tool(temp.path(), "echo", &["hello".to_owned()]).expect_err("asleep");
    assert!(
        asleep.to_string().contains("cannot run echo while sleep"),
        "{asleep}"
    );

    let woke = server.wake_phrase_for_test();
    assert!(woke.body.status().is_some(), "wake should apply: {woke:?}");

    let mut watcher = Client::connect(temp.path()).expect("watcher");
    watcher
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let ran = ctl::call_tool(temp.path(), "echo", &["hello".to_owned()]).expect("echo");
    assert_eq!(ran.state, VoiceState::Awake);
    assert_eq!(ran.message.as_deref(), Some("echo: hello"));
    assert_eq!(ran.detail.as_deref(), Some("echo: hello"));
    assert!(ctl::format_status(&ran).contains("echo: hello"));

    let pong = ctl::call_tool(temp.path(), "echo", &[]).expect("pong");
    assert_eq!(pong.message.as_deref(), Some("pong"));

    match watcher.read().expect("started") {
        ServerMessage::Event {
            body: Event::ToolStarted { name },
        } => assert_eq!(name, "echo"),
        other => panic!("expected tool_started, got {other:?}"),
    }
    match watcher.read().expect("finished") {
        ServerMessage::Event {
            body: Event::ToolFinished { name, detail },
        } => {
            assert_eq!(name, "echo");
            assert_eq!(detail.as_deref(), Some("echo: hello"));
        }
        other => panic!("expected tool_finished, got {other:?}"),
    }
    match watcher.read().expect("pong started") {
        ServerMessage::Event {
            body: Event::ToolStarted { name },
        } => assert_eq!(name, "echo"),
        other => panic!("expected tool_started, got {other:?}"),
    }
    match watcher.read().expect("pong finished") {
        ServerMessage::Event {
            body: Event::ToolFinished { detail, .. },
        } => assert_eq!(detail.as_deref(), Some("pong")),
        other => panic!("expected tool_finished, got {other:?}"),
    }

    let unknown = ctl::call_tool(temp.path(), "volume", &[]).expect_err("unknown");
    assert!(
        unknown.to_string().contains("unknown tool: volume"),
        "{unknown}"
    );

    let hibernated = ctl::call(temp.path(), Command::Hibernate).expect("hibernate");
    assert_eq!(hibernated.state, VoiceState::Hibernate);
    match watcher.read().expect("state") {
        ServerMessage::Event {
            body: Event::StateChanged { state, .. },
        } => assert_eq!(state, VoiceState::Hibernate),
        other => panic!("expected state_changed after the tool events, got {other:?}"),
    }

    let refused = ctl::call_tool(temp.path(), "echo", &[]).expect_err("hibernate");
    assert!(
        refused
            .to_string()
            .contains("cannot run echo while hibernate"),
        "{refused}"
    );

    drop(watcher);
    drop(server);
}

#[test]
fn ctl_notify_confirms_once_and_cancel_does_not_run() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");
    let woke = server.wake_phrase_for_test();
    assert!(woke.body.status().is_some(), "wake should apply: {woke:?}");

    let mut watcher = Client::connect(temp.path()).expect("watcher");
    watcher
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let pending = ctl::call_tool(temp.path(), "notify", &["hello".to_owned()]).expect("pending");
    assert_eq!(
        pending.message.as_deref(),
        Some("pending confirmation 1 for notify")
    );
    let waiting = pending.pending_tool.clone().expect("pending tool");
    assert_eq!(waiting.pending_id, "1");
    assert_eq!(waiting.args, ["hello"]);
    let printed = ctl::format_status(&pending);
    assert!(printed.contains("pending: 1 notify hello"), "{printed}");
    assert!(
        printed.contains("last tool: notify confirm pending"),
        "{printed}"
    );
    match watcher.read().expect("pending event") {
        ServerMessage::Event {
            body: Event::ToolConfirmPending {
                pending_id, name, ..
            },
        } => {
            assert_eq!(pending_id, "1");
            assert_eq!(name, "notify");
        }
        other => panic!("expected tool_confirm_pending, got {other:?}"),
    }

    let confirmed = ctl::call_confirm(temp.path(), "1").expect("confirm");
    assert_eq!(confirmed.message.as_deref(), Some("hello"));
    assert!(confirmed.pending_tool.is_none());
    assert_eq!(
        confirmed.last_tool.as_deref(),
        Some("notify confirm confirmed")
    );
    match watcher.read().expect("resolved") {
        ServerMessage::Event {
            body:
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
        } => assert_eq!(pending_id, "1"),
        other => panic!("expected tool_confirm_resolved, got {other:?}"),
    }
    match watcher.read().expect("started") {
        ServerMessage::Event {
            body: Event::ToolStarted { name },
        } => assert_eq!(name, "notify"),
        other => panic!("expected tool_started, got {other:?}"),
    }
    match watcher.read().expect("finished") {
        ServerMessage::Event {
            body: Event::ToolFinished { name, detail },
        } => {
            assert_eq!(name, "notify");
            assert_eq!(detail.as_deref(), Some("hello"));
        }
        other => panic!("expected tool_finished, got {other:?}"),
    }

    let spent = ctl::call_confirm(temp.path(), "1").expect_err("spent");
    assert!(
        spent
            .to_string()
            .contains("unknown pending confirmation: 1"),
        "{spent}"
    );

    drop(watcher);
    drop(server);
}

#[test]
fn ctl_cancel_and_hibernate_clear_a_pending_notification() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");
    assert!(
        server.wake_phrase_for_test().body.status().is_some(),
        "wake should apply"
    );

    let mut watcher = Client::connect(temp.path()).expect("watcher");
    watcher
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let staged = ctl::call_tool(temp.path(), "notify", &["nope".to_owned()]).expect("stage");
    assert_eq!(
        staged
            .pending_tool
            .as_ref()
            .map(|pending| pending.pending_id.as_str()),
        Some("1")
    );
    watcher.read().expect("pending");
    let cancelled = ctl::call_cancel(temp.path(), "1").expect("cancel");
    assert_eq!(cancelled.message.as_deref(), Some("cancelled 1"));
    assert!(cancelled.pending_tool.is_none());
    match watcher.read().expect("cancelled") {
        ServerMessage::Event {
            body: Event::ToolConfirmResolved {
                accepted: false, ..
            },
        } => {}
        other => panic!("expected a cancelled resolution, got {other:?}"),
    }
    let gone = ctl::call_confirm(temp.path(), "1").expect_err("gone");
    assert!(
        gone.to_string().contains("unknown pending confirmation: 1"),
        "{gone}"
    );

    let denied = ctl::call_tool(temp.path(), "shell", &[]).expect_err("shell");
    assert!(
        denied.to_string().contains("tool denied: shell"),
        "{denied}"
    );

    ctl::call_tool(temp.path(), "notify", &["later".to_owned()]).expect("later");
    watcher.read().expect("third pending");
    let hibernated = ctl::call(temp.path(), Command::Hibernate).expect("hibernate");
    assert_eq!(hibernated.state, VoiceState::Hibernate);
    assert!(hibernated.pending_tool.is_none());
    match watcher.read().expect("state") {
        ServerMessage::Event {
            body: Event::StateChanged { state, .. },
        } => assert_eq!(state, VoiceState::Hibernate),
        other => panic!("expected state_changed, got {other:?}"),
    }
    match watcher.read().expect("cleared") {
        ServerMessage::Event {
            body:
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: false,
                },
        } => assert_eq!(pending_id, "2"),
        other => panic!("expected the pending call to clear, got {other:?}"),
    }
    let stale = ctl::call_confirm(temp.path(), "2").expect_err("stale");
    assert!(
        stale
            .to_string()
            .contains("unknown pending confirmation: 2"),
        "{stale}"
    );

    drop(watcher);
    drop(server);
}

#[test]
fn ctl_email_send_confirms_once() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");
    let woke = server.wake_phrase_for_test();
    assert!(woke.body.status().is_some(), "wake should apply: {woke:?}");

    let mut watcher = Client::connect(temp.path()).expect("watcher");
    watcher
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let args = vec![
        "ada@example.com".to_owned(),
        "hello".to_owned(),
        "a short note".to_owned(),
    ];
    let pending = ctl::call_tool(temp.path(), "email_send", &args).expect("pending");
    assert_eq!(
        pending.message.as_deref(),
        Some("pending confirmation 1 for email_send")
    );
    let waiting = pending.pending_tool.clone().expect("pending tool");
    assert_eq!(waiting.pending_id, "1");
    assert_eq!(waiting.name, "email_send");
    assert_eq!(waiting.args, args);
    let printed = ctl::format_status(&pending);
    assert!(
        printed.contains("pending: 1 email_send ada@example.com hello a short note"),
        "{printed}"
    );
    assert!(
        printed.contains("last tool: email_send confirm pending"),
        "{printed}"
    );
    match watcher.read().expect("pending event") {
        ServerMessage::Event {
            body: Event::ToolConfirmPending {
                pending_id, name, ..
            },
        } => {
            assert_eq!(pending_id, "1");
            assert_eq!(name, "email_send");
        }
        other => panic!("expected tool_confirm_pending, got {other:?}"),
    }

    let confirmed = ctl::call_confirm(temp.path(), "1").expect("confirm");
    assert_eq!(confirmed.message.as_deref(), Some("sent 1"));
    assert!(confirmed.pending_tool.is_none());
    assert_eq!(
        confirmed.last_tool.as_deref(),
        Some("email_send confirm confirmed")
    );
    match watcher.read().expect("resolved") {
        ServerMessage::Event {
            body:
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
        } => assert_eq!(pending_id, "1"),
        other => panic!("expected tool_confirm_resolved, got {other:?}"),
    }
    match watcher.read().expect("started") {
        ServerMessage::Event {
            body: Event::ToolStarted { name },
        } => assert_eq!(name, "email_send"),
        other => panic!("expected tool_started, got {other:?}"),
    }
    match watcher.read().expect("finished") {
        ServerMessage::Event {
            body: Event::ToolFinished { name, detail },
        } => {
            assert_eq!(name, "email_send");
            assert_eq!(detail.as_deref(), Some("sent 1"));
        }
        other => panic!("expected tool_finished, got {other:?}"),
    }

    let spent = ctl::call_confirm(temp.path(), "1").expect_err("spent");
    assert!(
        spent
            .to_string()
            .contains("unknown pending confirmation: 1"),
        "{spent}"
    );

    drop(watcher);
    drop(server);
}

#[test]
fn ctl_ask_is_refused_until_awake_then_returns_the_fixture_reply() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");
    server.install_chat_fixture_for_test(crate::chat::xai_key_fixture(
        true,
        Some("sk-test-secret"),
        "pong",
    ));

    let asleep = ctl::call_ask(temp.path(), "hello").expect_err("asleep");
    assert_eq!(
        asleep.to_string(),
        "ask while sleep (chat acts only while awake)"
    );
    assert!(server.chat_posts_for_test().is_empty());
    assert!(server.session_turns_for_test().is_empty());
    assert!(!asleep.to_string().contains("sk-test-secret"));

    let woke = server.wake_phrase_for_test();
    assert!(woke.body.status().is_some(), "wake should apply: {woke:?}");

    let mut watcher = Client::connect(temp.path()).expect("watcher");
    watcher
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");

    let asked = ctl::call_ask(temp.path(), "hello").expect("ask");
    assert_eq!(asked.state, VoiceState::Awake);
    assert_eq!(asked.message.as_deref(), Some("pong"));
    assert!(asked.detail.is_none());
    let printed = ctl::format_status(&asked);
    assert!(printed.contains("pong"), "{printed}");
    assert!(!printed.contains("sk-test-secret"), "{printed}");
    assert_eq!(server.session_turns_for_test(), vec!["hello".to_owned()]);

    let posts = server.chat_posts_for_test();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, "https://api.x.ai/v1/chat/completions");
    let body: serde_json::Value = serde_json::from_str(&posts[0].body).expect("json");
    assert_eq!(
        body["messages"][0]["content"].as_str(),
        server.session_instructions_for_test().as_deref()
    );
    assert_eq!(body["messages"][1]["content"].as_str(), Some("hello"));
    assert!(!posts[0].body.contains("sk-test-secret"));

    let quiet = watcher.read().expect_err("ask does not broadcast");
    assert!(
        quiet.to_string().contains("timed out"),
        "expected no event, got {quiet}"
    );

    let chatted = ctl::run(
        temp.path(),
        &ctl::CtlAction::Chat {
            text: "hello there".to_owned(),
        },
    )
    .expect("chat");
    assert!(chatted.contains("pong"), "{chatted}");
    assert!(!chatted.contains("sk-test-secret"), "{chatted}");
    let posts = server.chat_posts_for_test();
    assert_eq!(posts.len(), 2);
    let body: serde_json::Value = serde_json::from_str(&posts[1].body).expect("json");
    assert_eq!(body["messages"][1]["content"].as_str(), Some("hello there"));

    watcher
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let slept = ctl::call(temp.path(), Command::Sleep).expect("sleep");
    assert_eq!(slept.state, VoiceState::Sleep);
    match watcher.read().expect("state") {
        ServerMessage::Event {
            body: Event::StateChanged { state, .. },
        } => assert_eq!(state, VoiceState::Sleep),
        other => panic!("expected state_changed after ask, got {other:?}"),
    }

    drop(watcher);
    drop(server);
}

#[test]
fn ctl_ask_rejects_a_failed_test_and_a_missing_bearer() {
    let temp = TempSocket::new();
    let soul = TestSoulDir::valid();
    let server = serve::spawn(temp.path.clone(), soul.soul_dir()).expect("serve");
    let woke = server.wake_phrase_for_test();
    assert!(woke.body.status().is_some(), "wake should apply: {woke:?}");

    server.install_chat_fixture_for_test(crate::chat::xai_key_fixture(
        false,
        Some("sk-test-secret"),
        "pong",
    ));
    let failed = ctl::call_ask(temp.path(), "hello").expect_err("test");
    assert_eq!(
        failed.to_string(),
        "Test has not succeeded for xai-key. Run Test in Settings."
    );
    assert!(server.chat_posts_for_test().is_empty());
    assert!(server.session_turns_for_test().is_empty());
    assert!(!failed.to_string().contains("sk-test-secret"));

    server.install_chat_fixture_for_test(crate::chat::xai_key_fixture(true, None, "pong"));
    let missing = ctl::call_ask(temp.path(), "hello").expect_err("bearer");
    assert_eq!(missing.to_string(), "No xAI API key is configured.");
    assert!(server.chat_posts_for_test().is_empty());
    assert!(server.session_turns_for_test().is_empty());

    let hibernated = ctl::call(temp.path(), Command::Hibernate).expect("hibernate");
    assert_eq!(hibernated.state, VoiceState::Hibernate);
    let refused = ctl::call_ask(temp.path(), "hello").expect_err("hibernate");
    assert_eq!(
        refused.to_string(),
        "ask while hibernate (chat acts only while awake)"
    );
    assert!(server.chat_posts_for_test().is_empty());

    drop(server);
}
