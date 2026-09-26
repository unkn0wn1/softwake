use std::time::Duration;

use softwake_providers::{
    HttpResponse, ProviderHandle, ProviderId, ProviderSettings, SecretBag, TestReport,
};
use softwake_session::SessionPhase;
use softwake_state::{CooldownConfig, VoiceState};
use softwake_wake::PhraseTable;

use super::{COMMANDS, Demo};
use crate::chat::ChatFixture;
use crate::soul::TestSoulDir;

fn demo_with(cooldown: CooldownConfig) -> (Demo, TestSoulDir) {
    let soul = TestSoulDir::valid();
    let demo = Demo::new(PhraseTable::default(), cooldown, soul.soul_dir());
    (demo, soul)
}

fn no_cooldown() -> CooldownConfig {
    CooldownConfig {
        post_wake: Duration::ZERO,
        post_sleep: Duration::ZERO,
    }
}

fn report(ok: bool) -> TestReport {
    TestReport {
        ok,
        message: "recorded".to_owned(),
    }
}

fn xai_handle(
    model: &str,
    models: &[&str],
    test_ok: Option<bool>,
    key: Option<&str>,
) -> ProviderHandle {
    let mut settings = ProviderSettings {
        selected_provider: ProviderId::XaiKey,
        selected_model: model.to_owned(),
        ..ProviderSettings::default()
    };
    if !models.is_empty() {
        settings.store_models(
            ProviderId::XaiKey,
            models.iter().copied().map(str::to_owned).collect(),
            Vec::new(),
            1,
        );
    }
    if let Some(ok) = test_ok {
        settings.store_test(ProviderId::XaiKey, report(ok));
    }
    let mut bag = SecretBag::empty();
    bag.xai_api_key = key.map(str::to_owned);
    ProviderHandle::from_parts(settings, bag)
}

fn pong_body() -> HttpResponse {
    HttpResponse {
        status: 200,
        body: serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "pong"}}]
        })
        .to_string(),
    }
}

fn install(demo: &mut Demo, handle: ProviderHandle, response: HttpResponse) {
    demo.install_chat_fixture(ChatFixture::new(handle, true, response));
}

fn wake(demo: &mut Demo) {
    demo.handle_line("wake", Duration::ZERO);
}

#[test]
fn ask_while_asleep_does_not_post() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    let result = demo.handle_line("ask hello", Duration::ZERO);
    assert_eq!(
        result.lines[0],
        "rejected: ask while sleep (chat acts only while awake)"
    );
    assert_eq!(demo.session_phase(), SessionPhase::Closed);
    assert!(demo.session_turns().is_empty());
    assert!(demo.chat_posts().is_empty());
}

#[test]
fn chat_while_hibernating_names_hibernate() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    demo.handle_line("hibernate", Duration::ZERO);
    let result = demo.handle_line("chat hello", Duration::ZERO);
    assert_eq!(
        result.lines[0],
        "rejected: chat while hibernate (chat acts only while awake)"
    );
    assert_eq!(demo.state(), VoiceState::Hibernate);
    assert!(demo.chat_posts().is_empty());
    assert!(demo.session_turns().is_empty());
}

#[test]
fn ask_and_chat_need_text_before_the_voice_check() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let ask = demo.handle_line("ask", Duration::ZERO);
    assert_eq!(ask.lines[0], "rejected: ask needs text");
    assert_eq!(ask.lines[1], COMMANDS);
    let chat = demo.handle_line("chat", Duration::ZERO);
    assert_eq!(chat.lines[0], "rejected: chat needs text");
    assert_eq!(chat.lines[1], COMMANDS);
    assert_eq!(demo.session_phase(), SessionPhase::Closed);
}

#[test]
fn ask_while_awake_returns_the_fixture_reply() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    wake(&mut demo);
    let result = demo.handle_line("ask hello", Duration::ZERO);
    assert_eq!(result.lines[0], "assistant: pong");
    assert!(result.lines.iter().any(|line| line == "state: awake"));
    assert_eq!(demo.state(), VoiceState::Awake);
    assert_eq!(demo.session_turns(), vec!["hello".to_owned()]);
    let posts = demo.chat_posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, "https://api.x.ai/v1/chat/completions");
    let body: serde_json::Value = serde_json::from_str(&posts[0].body).expect("json");
    assert_eq!(
        body["messages"][0]["content"].as_str(),
        demo.session_instructions()
    );
    assert_eq!(body["messages"][1]["content"].as_str(), Some("hello"));
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(!result.lines.join("\n").contains("sk-test-secret"));
}

#[test]
fn chat_sends_the_rest_of_the_line() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    wake(&mut demo);
    let result = demo.handle_line("chat hello there", Duration::ZERO);
    assert_eq!(result.lines[0], "assistant: pong");
    assert_eq!(demo.session_turns(), vec!["hello there".to_owned()]);
    let body: serde_json::Value = serde_json::from_str(&demo.chat_posts()[0].body).expect("json");
    assert_eq!(body["messages"][1]["content"].as_str(), Some("hello there"));
}

#[test]
fn ask_command_word_is_case_insensitive() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    wake(&mut demo);
    let result = demo.handle_line("ASK hello", Duration::ZERO);
    assert_eq!(result.lines[0], "assistant: pong");
    assert_eq!(demo.session_turns(), vec!["hello".to_owned()]);
}

#[test]
fn awake_fixture_rejects_test_model_and_credential_without_a_post() {
    let cases = [
        (
            xai_handle(
                "grok-4.5",
                &["grok-4.5"],
                Some(false),
                Some("sk-test-secret"),
            ),
            "rejected: Test has not succeeded for xai-key. Run Test in Settings.",
        ),
        (
            xai_handle("", &["grok-4.5"], Some(true), Some("sk-test-secret")),
            "rejected: No chat model is selected. Choose one in Settings after Test.",
        ),
        (
            xai_handle(
                "not-cached",
                &["grok-4.5"],
                Some(true),
                Some("sk-test-secret"),
            ),
            "rejected: No chat model is selected. Choose one in Settings after Test.",
        ),
        (
            xai_handle("grok-4.5", &["grok-4.5"], Some(true), None),
            "rejected: No xAI API key is configured.",
        ),
    ];
    for (handle, line) in cases {
        let (mut demo, _soul) = demo_with(no_cooldown());
        install(&mut demo, handle, pong_body());
        wake(&mut demo);
        let result = demo.handle_line("ask hello", Duration::ZERO);
        assert_eq!(result.lines[0], line);
        assert!(demo.chat_posts().is_empty());
        assert!(demo.session_turns().is_empty());
        assert_eq!(demo.state(), VoiceState::Awake);
    }
}

#[test]
fn rejected_credentials_keep_the_user_line_and_hide_the_bearer() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        HttpResponse {
            status: 401,
            body: "sk-test-secret".to_owned(),
        },
    );
    wake(&mut demo);
    let result = demo.handle_line("ask hello", Duration::ZERO);
    assert_eq!(
        result.lines[0],
        "rejected: Provider rejected the credentials."
    );
    assert_eq!(demo.session_turns(), vec!["hello".to_owned()]);
    assert_eq!(demo.chat_posts().len(), 1);
    assert!(!result.lines.join("\n").contains("sk-test-secret"));
    assert_eq!(demo.state(), VoiceState::Awake);
}

#[test]
fn verbose_ask_names_the_provider_and_model_without_the_bearer() {
    let (demo, _soul) = demo_with(no_cooldown());
    let mut demo = demo.with_verbose(true);
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    wake(&mut demo);
    let result = demo.handle_line("ask hello", Duration::ZERO);
    let text = result.lines.join("\n");
    assert!(text.contains("verbose: parsed: ask"));
    assert!(text.contains("verbose: provider: xai-key"));
    assert!(text.contains("verbose: model: grok-4.5"));
    assert!(!text.contains("sk-test-secret"));
    assert_eq!(demo.session_turns(), vec!["hello".to_owned()]);
}

#[test]
fn sleep_after_ask_refuses_the_next_ask_without_a_post() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    wake(&mut demo);
    demo.handle_line("ask hello", Duration::ZERO);
    assert_eq!(demo.chat_posts().len(), 1);
    demo.handle_line("sleep", Duration::ZERO);
    let again = demo.handle_line("ask again", Duration::ZERO);
    assert_eq!(
        again.lines[0],
        "rejected: ask while sleep (chat acts only while awake)"
    );
    assert_eq!(demo.chat_posts().len(), 1);
    assert_eq!(demo.session_phase(), SessionPhase::Closed);
}

#[test]
fn ask_with_mock_memory_appends_budgeted_hits_after_the_pack() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    let mut memory = softwake_memory::MockMemory::enabled();
    memory
        .remember("garage code is on the hook")
        .expect("remember garage");
    memory
        .remember("wifi password is secret")
        .expect("remember wifi");
    memory
        .remember("garage door opens at dusk")
        .expect("remember dusk");
    demo.install_memory_fixture(memory);
    wake(&mut demo);
    let result = demo.handle_line("ask garage", Duration::ZERO);
    assert_eq!(result.lines[0], "assistant: pong");
    let body: serde_json::Value = serde_json::from_str(&demo.chat_posts()[0].body).expect("json");
    let system = body["messages"][0]["content"].as_str().expect("system");
    let pack = demo.session_instructions().expect("pack");
    assert!(system.starts_with(pack));
    assert!(system.contains(softwake_memory::RECALL_LEAD));
    assert!(system.contains("garage code is on the hook"));
    assert!(system.contains("garage door opens at dusk"));
    assert!(!system.contains("wifi password"));
    assert_eq!(body["messages"][1]["content"].as_str(), Some("garage"));
}

#[test]
fn ask_with_empty_query_hits_or_disabled_memory_leaves_pack_alone() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    // Disabled memory: fail-open, pack unchanged.
    demo.install_memory_fixture(softwake_memory::MockMemory::default());
    wake(&mut demo);
    let result = demo.handle_line("ask hello", Duration::ZERO);
    assert_eq!(result.lines[0], "assistant: pong");
    let body: serde_json::Value = serde_json::from_str(&demo.chat_posts()[0].body).expect("json");
    assert_eq!(
        body["messages"][0]["content"].as_str(),
        demo.session_instructions()
    );
}

#[test]
fn ask_with_file_memory_temp_dir_attaches_substring_hits() {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("softwake-daemon-memory-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(softwake_memory::MEMORY_FILE_NAME);
    {
        let mut memory = softwake_memory::FileMemory::open_enabled(&path).expect("open");
        memory.remember("alpha token one").expect("a");
        memory.remember("beta other").expect("b");
        memory.remember("alpha token two").expect("c");
    }

    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    // Drive the explicit path helper, then install matching MockMemory so ask
    // sees the same appendix the disk helper would produce for that file.
    let appendix = crate::chat::memory_appendix_at_path(&path, "alpha");
    assert!(appendix.contains("alpha token one"));
    assert!(appendix.contains("alpha token two"));
    assert!(!appendix.contains("beta other"));
    assert!(crate::chat::memory_appendix_at_path(&path, "").is_empty());
    assert!(crate::chat::memory_appendix_at_path(&dir.join("missing.json"), "alpha").is_empty());

    let mut memory = softwake_memory::MockMemory::enabled();
    memory.remember("alpha token one").expect("a");
    memory.remember("beta other").expect("b");
    memory.remember("alpha token two").expect("c");
    demo.install_memory_fixture(memory);
    wake(&mut demo);
    let _ = demo.handle_line("ask alpha", Duration::ZERO);
    let body: serde_json::Value = serde_json::from_str(&demo.chat_posts()[0].body).expect("json");
    let system = body["messages"][0]["content"].as_str().expect("system");
    assert_eq!(
        system,
        &softwake_session::assemble_system(demo.session_instructions().expect("pack"), &appendix,)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn second_ask_replays_prior_user_and_assistant() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    install(
        &mut demo,
        xai_handle(
            "grok-4.5",
            &["grok-4.5"],
            Some(true),
            Some("sk-test-secret"),
        ),
        pong_body(),
    );
    wake(&mut demo);
    assert_eq!(
        demo.handle_line("ask remember blue", Duration::ZERO).lines[0],
        "assistant: pong"
    );
    let result = demo.handle_line("ask what color", Duration::ZERO);
    assert_eq!(result.lines[0], "assistant: pong");
    assert_eq!(
        demo.session_turns(),
        vec!["remember blue".to_owned(), "what color".to_owned()]
    );
    let body: serde_json::Value = serde_json::from_str(&demo.chat_posts()[1].body).expect("json");
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 4); // system + user + assistant + user
    assert_eq!(messages[1]["content"], "remember blue");
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["content"], "pong");
    assert_eq!(messages[3]["content"], "what color");
}

#[test]
fn low_context_limit_compacts_before_ask() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let mut settings = ProviderSettings {
        selected_provider: ProviderId::XaiKey,
        selected_model: "grok-4.5".to_owned(),
        context_limit_tokens: 80,
        compact_at_percent: 10,
        keep_recent_turns: 2,
        ..ProviderSettings::default()
    };
    settings.store_models(
        ProviderId::XaiKey,
        vec!["grok-4.5".to_owned()],
        Vec::new(),
        1,
    );
    settings.store_test(
        ProviderId::XaiKey,
        TestReport {
            ok: true,
            message: "recorded".to_owned(),
        },
    );
    let mut bag = SecretBag::empty();
    bag.xai_api_key = Some("sk-test-secret".to_owned());
    install(
        &mut demo,
        ProviderHandle::from_parts(settings, bag),
        pong_body(),
    );
    wake(&mut demo);
    for i in 0..4 {
        let line = format!("ask turn-{i}-with-enough-text-to-grow-history");
        let result = demo.handle_line(&line, Duration::ZERO);
        assert!(
            result.lines.iter().any(|l| l.starts_with("assistant:")),
            "{result:?}"
        );
    }
    let posts = demo.chat_posts();
    assert!(
        posts.len() >= 5,
        "expected compact + asks, got {}",
        posts.len()
    );
    let last: serde_json::Value = serde_json::from_str(&posts.last().unwrap().body).expect("json");
    let contents: Vec<&str> = last["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect();
    assert!(
        contents.iter().any(|c| c.starts_with("Session summary:")),
        "expected summary in {contents:?}"
    );
}
