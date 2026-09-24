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
use crate::runtime::RELOAD_MESSAGE;
use crate::serve;

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
    let server = serve::spawn(temp.path.clone()).expect("serve");

    let status = ctl::call(temp.path(), Command::GetStatus).expect("status");
    assert_eq!(status.state, VoiceState::Sleep);
    assert!(status.capture_running);
    assert!(!status.soul_reload_pending);

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
fn reload_soul_is_a_stub_and_a_second_client_sees_state_changed() {
    let temp = TempSocket::new();
    let server = serve::spawn(temp.path.clone()).expect("serve");

    let mut watcher = Client::connect(temp.path()).expect("watcher");
    watcher
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let reloaded = ctl::call(temp.path(), Command::ReloadSoul).expect("reload");
    assert!(reloaded.soul_reload_pending);
    assert_eq!(reloaded.message.as_deref(), Some(RELOAD_MESSAGE));
    assert_eq!(
        ctl::format_status(&reloaded),
        format!("state: sleep\ncapture: running\nsoul reload: pending\n{RELOAD_MESSAGE}\n")
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
    let server = serve::spawn(temp.path.clone()).expect("serve");
    let error = serve::spawn(temp.path.clone()).expect_err("second");
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

    let restarted = serve::spawn(temp.path.clone()).expect("restart");
    drop(restarted);
}

impl TempSocket {
    fn path(&self) -> &Path {
        &self.path
    }
}
