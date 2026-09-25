//! Command line for `softwaked`.
//!
//! `demo` stays a typed loop. `serve` and `ctl` are the socket modes. `--help`
//! covers all of them. A usage error exits with status 2; a daemon or socket
//! error exits with status 1.

use std::env;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use softwake_ipc::resolve_socket_path;
use softwake_soul::resolve_soul_dir;
use softwake_state::{CooldownConfig, Machine};
use softwake_wake::PhraseTable;

use crate::capture::{self, CaptureKind};
use crate::ctl::CtlAction;
use crate::demo::Demo;
use crate::{ctl, serve};

pub fn execute<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    match parse_args(args) {
        Ok(Mode::Status) => {
            println!("{}", status_line());
            ExitCode::SUCCESS
        }
        Ok(Mode::Demo { verbose, soul_dir }) => {
            let verbose = verbose || debug_log_requested(env::var("SOFTWAKE_LOG").ok().as_deref());
            run_demo(verbose, soul_dir.as_deref())
        }
        Ok(Mode::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Mode::Serve {
            socket,
            soul_dir,
            capture,
        }) => {
            let dir = match resolve_soul_dir(soul_dir.as_deref()) {
                Ok(dir) => dir,
                Err(error) => {
                    eprintln!("softwaked: {error}");
                    return ExitCode::from(1);
                }
            };
            match serve::run(socket.as_deref(), dir, capture) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("softwaked: {error}");
                    ExitCode::from(1)
                }
            }
        }
        Ok(Mode::Ctl { socket, command }) => run_ctl(socket.as_deref(), &command),
        Err(message) => {
            eprintln!("softwaked: {message}");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn run_ctl(socket: Option<&Path>, command: &CtlAction) -> ExitCode {
    let path = match resolve_socket_path(socket) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("softwaked: {error}");
            return ExitCode::from(1);
        }
    };
    match ctl::run(&path, command) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("softwaked: {error}");
            ExitCode::from(1)
        }
    }
}

fn status_line() -> String {
    let machine = Machine::new(CooldownConfig::default());
    format!("softwaked state: {}", machine.state())
}

fn run_demo(verbose: bool, soul_dir: Option<&Path>) -> ExitCode {
    let dir = match resolve_soul_dir(soul_dir) {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("softwaked: {error}");
            return ExitCode::from(1);
        }
    };
    let mut demo =
        Demo::new(PhraseTable::default(), CooldownConfig::default(), dir).with_verbose(verbose);
    for line in demo.banner_lines() {
        println!("{line}");
    }
    let mut last = Instant::now();
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut buffer = String::new();
    loop {
        // No newline on the prompt, so flush before blocking on stdin. A pipe
        // is block-buffered and would otherwise hide the prompt.
        print!("> ");
        if let Err(error) = io::stdout().flush() {
            eprintln!("softwaked: stdout: {error}");
            return ExitCode::from(1);
        }
        buffer.clear();
        let read = match reader.read_line(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                eprintln!("softwaked: stdin: {error}");
                return ExitCode::from(1);
            }
        };
        if read == 0 {
            return ExitCode::SUCCESS;
        }
        strip_line_ending(&mut buffer);
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(last);
        last = now;
        let result = demo.handle_line(&buffer, elapsed);
        for output in result.lines {
            println!("{output}");
        }
        if result.quit {
            return ExitCode::SUCCESS;
        }
    }
}

/// Drop the delimiter [`std::io::BufRead::lines`] would have dropped.
fn strip_line_ending(buffer: &mut String) {
    if buffer.ends_with('\n') {
        buffer.pop();
        if buffer.ends_with('\r') {
            buffer.pop();
        }
    }
}

fn debug_log_requested(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("debug"))
}

fn help_text() -> &'static str {
    r#"softwaked — Softwake daemon

Usage:
  softwaked                 print the initial voice state and exit
  softwaked demo            typed-command voice-state demo
  softwaked demo --verbose  per-line phrase and transition detail
  softwaked demo -v         same as --verbose
  softwaked --demo          same as demo (--verbose and -v work here too)
  softwaked demo --soul-dir PATH
  softwaked serve           listen for ctl and UI clients
  softwaked --serve         same as serve
  softwaked serve --socket PATH
  softwaked serve --soul-dir PATH
  softwaked serve --capture mock|pipewire
  softwaked ctl status      print the running daemon's voice state
  softwaked ctl hibernate   hibernate (stop capture)
  softwaked ctl resume      leave hibernate and land in sleep
  softwaked ctl wake        enter awake from sleep (valid soul pack required)
  softwaked ctl sleep       sleep from awake
  softwaked ctl reload-soul re-read the soul pack; it applies on the next awake
  softwaked ctl tool NAME [ARG...]
                            run one tool while awake, or stage a confirm-gated tool
  softwaked ctl confirm-tool ID
                            run the pending confirm-gated tool
  softwaked ctl cancel-tool ID
                            drop the pending confirmation without running it
  softwaked ctl ask TEXT... send one line while the daemon is awake
  softwaked ctl chat TEXT...
                            same socket message as ctl ask
  softwaked --help          print this help

The demo reads typed commands only and prints "> " before each line.
It uses mock capture. A microphone is not opened. Native PipeWire is an
optional feature (pipewire-native) and is not linked in the default build.
SOFTWAKE_LOG=debug enables the same detail as --verbose and -v.

Demo commands, one per line:
  wake, sleep, hibernate, resume, status, reload-soul, tool, confirm, cancel, hear, say, ask, chat, quit

`hear TEXT` injects a mock transcript while awake. `say TEXT` records mock speech.
`ask TEXT` and `chat TEXT` send one typed line to the selected provider while
awake. Sleep and hibernate refuse those commands. The default build does not
open a provider socket for `ask` or `chat`. `ctl ask` and `ctl chat` send that
same turn to a running `softwaked serve`.

`tool echo` returns pong. `tool echo hello` returns "echo: hello".
`tool notify hello` waits until `confirm` (or `confirm-tool ID`).
`tool email_send TO SUBJECT BODY...` waits the same way. Confirm appends
one in-memory message. `cancel` drops that pending call. `tool shell` is denied.
Other tool names are rejected. Sleep and hibernate reject every tool
and drop a pending confirmation. Safe tools run only while awake.

A wake phrase enters awake only when soul.md, user.md, rules.md, and
glossary.md are present, non-empty, and valid UTF-8. glossary.md must
parse as an alias map. Each file is at most 1 MiB. Hibernate, sleep,
and resume still run when the pack is missing. reload-soul reads the
files again; the new text applies on the next awake.

Serve owns the voice-state machine and capture. Default capture is mock.
`--capture pipewire` (or SOFTWAKE_CAPTURE=pipewire) opens the default
microphone when the binary was built with `--features pipewire-capture`.
It speaks newline-delimited JSON (protocol 1) on a Unix socket. A stale socket
file is removed on startup. If another serve is already listening, startup
fails and leaves that socket in place.

Socket path, first match wins:
  --socket PATH
  SOFTWAKE_SOCKET
  $XDG_RUNTIME_DIR/softwake/softwaked.sock
  /tmp/softwake-$UID/softwaked.sock when XDG_RUNTIME_DIR is unset

Soul directory, first match wins:
  --soul-dir PATH
  SOFTWAKE_SOUL_DIR
  $XDG_CONFIG_HOME/softwake/soul
  ~/.config/softwake/soul when XDG_CONFIG_HOME is unset

ctl exits non-zero when the daemon rejects the command or cannot be reached.
reload-soul re-reads the soul pack from disk. The daemon keeps the pack it
already applied until the next awake session. Status reports whether that
read is ok or missing.

`ctl ask TEXT` and `ctl chat TEXT` send one typed line to the running daemon.
Both use the same socket message. The daemon must already be awake. `ctl wake`
enters awake when the four-file pack is valid. `ctl resume` lands in sleep and
does not enter awake. Serve does not wake from the microphone yet (NullDetector until KWS weights).
The default build does not call the provider. A live answer needs the daemon
built with live-http. The assistant text is printed after the status lines.
A refusal exits non-zero.

`ctl tool` asks the running daemon to run one tool while it is awake.
`echo` runs immediately. `notify` and `email_send` return a pending id and
do not run until `ctl confirm-tool ID`. Confirming `email_send` appends one
in-memory message. `ctl cancel-tool ID` drops the pending call. `shell` is
denied. The result is printed after the status lines. A refusal names
the reason. `ctl wake` enters awake when the four-file pack is valid.
`ctl resume` still lands in sleep. Serve does not wake from the microphone."#
}

fn print_help() {
    println!("{}", help_text());
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Status,
    Demo {
        verbose: bool,
        soul_dir: Option<PathBuf>,
    },
    Help,
    Serve {
        socket: Option<PathBuf>,
        soul_dir: Option<PathBuf>,
        capture: CaptureKind,
    },
    Ctl {
        socket: Option<PathBuf>,
        command: CtlAction,
    },
}

fn parse_args<I>(args: I) -> Result<Mode, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let first = args.next();
    match first.as_deref() {
        None => Ok(Mode::Status),
        Some("demo" | "--demo") => parse_demo_flags(args),
        Some("help" | "--help" | "-h") => match args.next() {
            None => Ok(Mode::Help),
            Some(extra) => Err(format!("help does not take arguments (got {extra})")),
        },
        Some("serve" | "--serve") => parse_serve_flags(args),
        Some("ctl") => parse_ctl(args),
        Some(other) => Err(format!("unknown argument {other}")),
    }
}

fn parse_demo_flags(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let mut verbose = false;
    let mut soul_dir = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--verbose" | "-v" => verbose = true,
            "--soul-dir" => take_value("demo", "--soul-dir", &mut args, &mut soul_dir)?,
            other => return Err(format!("unknown demo argument {other}")),
        }
    }
    Ok(Mode::Demo { verbose, soul_dir })
}

fn parse_serve_flags(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let mut socket = None;
    let mut soul_dir = None;
    let mut capture_flag = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => take_value("serve", "--socket", &mut args, &mut socket)?,
            "--soul-dir" => take_value("serve", "--soul-dir", &mut args, &mut soul_dir)?,
            "--capture" => {
                if capture_flag.is_some() {
                    return Err("serve accepts one --capture".to_owned());
                }
                let Some(value) = args.next() else {
                    return Err("--capture needs mock or pipewire".to_owned());
                };
                capture_flag = Some(value);
            }
            other => return Err(format!("unknown serve argument {other}")),
        }
    }
    let capture = capture::resolve_capture_kind(
        capture_flag.as_deref(),
        env::var("SOFTWAKE_CAPTURE").ok().as_deref(),
    )?;
    Ok(Mode::Serve {
        socket,
        soul_dir,
        capture,
    })
}

fn parse_ctl(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let args: Vec<String> = args.into_iter().collect();
    let mut socket = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--socket" {
            if socket.is_some() {
                return Err("ctl accepts one --socket".to_owned());
            }
            let Some(path) = args.get(index + 1) else {
                return Err("--socket needs a path".to_owned());
            };
            if path.is_empty() {
                return Err("--socket needs a path".to_owned());
            }
            socket = Some(PathBuf::from(path));
            index += 2;
            continue;
        }
        positional.push(args[index].clone());
        index += 1;
    }
    let command = ctl_command(&positional)?;
    Ok(Mode::Ctl { socket, command })
}

fn ctl_command(positional: &[String]) -> Result<CtlAction, String> {
    match positional {
        [] => Err(
            "ctl needs a command: status, hibernate, resume, wake, sleep, reload-soul, tool, confirm-tool, cancel-tool, ask, chat"
                .to_owned(),
        ),
        [name] if name == "tool" => Err("ctl tool needs a tool name".to_owned()),
        [name] if name == "confirm-tool" || name == "cancel-tool" => {
            Err(format!("ctl {name} needs a pending id"))
        }
        [name] if name == "ask" || name == "chat" => Err(format!("ctl {name} needs text")),
        [name, tool_name, tool_args @ ..] if name == "tool" => Ok(CtlAction::Tool {
            name: tool_name.to_ascii_lowercase(),
            args: tool_args.to_vec(),
        }),
        [name, pending_id] if name == "confirm-tool" => Ok(CtlAction::ConfirmTool {
            pending_id: pending_id.clone(),
        }),
        [name, pending_id] if name == "cancel-tool" => Ok(CtlAction::CancelTool {
            pending_id: pending_id.clone(),
        }),
        [name, _, extra, ..] if name == "confirm-tool" || name == "cancel-tool" => {
            Err(format!("ctl {name} takes one pending id (got extra {extra})"))
        }
        [name, words @ ..] if name == "ask" || name == "chat" => {
            let text = words.join(" ");
            if text.trim().is_empty() {
                return Err(format!("ctl {name} needs text"));
            }
            if name == "ask" {
                Ok(CtlAction::Ask { text })
            } else {
                Ok(CtlAction::Chat { text })
            }
        }
        [name] => CtlAction::parse(name).ok_or_else(|| format!("unknown ctl argument {name}")),
        [first, second, ..] => {
            if CtlAction::parse(first).is_none() {
                return Err(format!("unknown ctl argument {first}"));
            }
            if CtlAction::parse(second).is_some() || second == "tool" {
                return Err("ctl takes one command".to_owned());
            }
            Err(format!("unknown ctl argument {second}"))
        }
    }
}

fn take_value(
    mode: &str,
    flag: &str,
    args: &mut impl Iterator<Item = String>,
    slot: &mut Option<PathBuf>,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{mode} accepts one {flag}"));
    }
    let Some(path) = args.next() else {
        return Err(format!("{flag} needs a path"));
    };
    if path.is_empty() {
        return Err(format!("{flag} needs a path"));
    }
    *slot = Some(PathBuf::from(path));
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::capture::CaptureKind;
    use std::path::PathBuf;

    use super::{Mode, debug_log_requested, help_text, parse_args, status_line, strip_line_ending};
    use crate::ctl::CtlAction;

    #[test]
    fn no_args_is_the_status_mode() {
        assert_eq!(parse_args(std::iter::empty::<String>()), Ok(Mode::Status));
        assert_eq!(status_line(), "softwaked state: sleep");
    }

    #[test]
    fn demo_and_help_flags() {
        assert_eq!(
            parse_args(["demo".to_owned()]),
            Ok(Mode::Demo {
                verbose: false,
                soul_dir: None
            })
        );
        assert_eq!(
            parse_args(["--demo".to_owned()]),
            Ok(Mode::Demo {
                verbose: false,
                soul_dir: None
            })
        );
        assert_eq!(parse_args(["--help".to_owned()]), Ok(Mode::Help));
        assert_eq!(parse_args(["-h".to_owned()]), Ok(Mode::Help));
    }

    #[test]
    fn demo_verbose_flags() {
        assert_eq!(
            parse_args(["demo".to_owned(), "--verbose".to_owned()]),
            Ok(Mode::Demo {
                verbose: true,
                soul_dir: None
            })
        );
        assert_eq!(
            parse_args(["demo".to_owned(), "-v".to_owned()]),
            Ok(Mode::Demo {
                verbose: true,
                soul_dir: None
            })
        );
        assert_eq!(
            parse_args(["--demo".to_owned(), "-v".to_owned()]),
            Ok(Mode::Demo {
                verbose: true,
                soul_dir: None
            })
        );
        assert_eq!(
            parse_args(["demo".to_owned(), "-v".to_owned(), "--verbose".to_owned()]),
            Ok(Mode::Demo {
                verbose: true,
                soul_dir: None
            })
        );
    }

    #[test]
    fn unknown_and_extra_arguments_are_errors() {
        assert!(parse_args(["--nope".to_owned()]).is_err());
        assert!(parse_args(["-v".to_owned()]).is_err());
        assert!(parse_args(["demo".to_owned(), "--extra".to_owned()]).is_err());
        assert!(parse_args(["demo".to_owned(), "-v".to_owned(), "--extra".to_owned()]).is_err());
        assert!(parse_args(["--demo".to_owned(), "--nope".to_owned()]).is_err());
        assert!(parse_args(["help".to_owned(), "-v".to_owned()]).is_err());
    }

    #[test]
    fn debug_log_env_selects_the_verbose_path() {
        assert!(debug_log_requested(Some("debug")));
        assert!(debug_log_requested(Some("  debug  ")));
        assert!(!debug_log_requested(Some("info")));
        assert!(!debug_log_requested(Some("DEBUG")));
        assert!(!debug_log_requested(None));
    }

    #[test]
    fn help_mentions_typed_only_verbose_and_the_log_env() {
        let help = help_text();
        assert!(help.contains("typed commands only"));
        assert!(help.contains("PipeWire"));
        assert!(help.contains("--verbose"));
        assert!(help.contains("-v"));
        assert!(help.contains("SOFTWAKE_LOG=debug"));
        assert!(help.contains("> "));
    }

    #[test]
    fn help_mentions_serve_ctl_and_the_socket() {
        let help = help_text();
        assert!(help.contains("softwaked serve"));
        assert!(help.contains("--serve"));
        assert!(help.contains("ctl status"));
        assert!(help.contains("ctl resume"));
        assert!(help.contains("ctl wake"));
        assert!(help.contains("valid soul pack"));
        assert!(help.contains("lands in sleep"));
        assert!(help.contains("reload-soul"));
        assert!(help.contains("SOFTWAKE_SOCKET"));
        assert!(help.contains("XDG_RUNTIME_DIR"));
        assert!(help.contains("/tmp/softwake-$UID/softwaked.sock"));
        assert!(help.contains("stale socket"));
        assert!(help.contains("newline-delimited JSON"));
        assert!(help.contains("--soul-dir"));
        assert!(help.contains("SOFTWAKE_SOUL_DIR"));
        assert!(help.contains("XDG_CONFIG_HOME"));
        assert!(help.contains("~/.config/softwake/soul"));
        assert!(help.contains("valid UTF-8"));
        assert!(help.contains("rules.md"));
        assert!(help.contains("glossary.md"));
        assert!(help.contains("alias map"));
        assert!(help.contains("1 MiB"));
        assert!(help.contains("applies on the next awake"));
        assert!(help.contains("ctl ask"));
        assert!(help.contains("ctl chat"));
        assert!(help.contains("live-http"));
        assert!(help.contains("ctl tool"));
        assert!(help.contains("tool echo"));
        assert!(help.contains("pong"));
        assert!(help.contains("confirm-tool"));
        assert!(help.contains("cancel-tool"));
        assert!(help.contains("notify"));
        assert!(help.contains("email_send"));
    }

    #[test]
    fn serve_and_ctl_flags() {
        assert_eq!(
            parse_args(["serve".to_owned()]),
            Ok(Mode::Serve {
                socket: None,
                soul_dir: None,
                capture: CaptureKind::Mock
            })
        );
        assert_eq!(
            parse_args([
                "--serve".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned(),
                "--soul-dir".to_owned(),
                "/tmp/soul".to_owned()
            ]),
            Ok(Mode::Serve {
                socket: Some(PathBuf::from("/tmp/sw.sock")),
                soul_dir: Some(PathBuf::from("/tmp/soul")),
                capture: CaptureKind::Mock
            })
        );
        assert_eq!(
            parse_args([
                "demo".to_owned(),
                "--soul-dir".to_owned(),
                "/tmp/soul".to_owned(),
                "-v".to_owned()
            ]),
            Ok(Mode::Demo {
                verbose: true,
                soul_dir: Some(PathBuf::from("/tmp/soul"))
            })
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "resume".to_owned()]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::Resume
            })
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "wake".to_owned()]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::Wake
            })
        );
        assert_eq!(
            parse_args([
                "ctl".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned(),
                "wake".to_owned()
            ]),
            Ok(Mode::Ctl {
                socket: Some(PathBuf::from("/tmp/sw.sock")),
                command: CtlAction::Wake
            })
        );
        assert_eq!(
            parse_args([
                "ctl".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned(),
                "reload-soul".to_owned()
            ]),
            Ok(Mode::Ctl {
                socket: Some(PathBuf::from("/tmp/sw.sock")),
                command: CtlAction::ReloadSoul
            })
        );
        assert_eq!(
            parse_args([
                "ctl".to_owned(),
                "status".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned()
            ]),
            Ok(Mode::Ctl {
                socket: Some(PathBuf::from("/tmp/sw.sock")),
                command: CtlAction::Status
            })
        );
    }

    #[test]
    fn serve_and_ctl_reject_incomplete_arguments() {
        assert!(parse_args(["serve".to_owned(), "--socket".to_owned()]).is_err());
        assert!(parse_args(["serve".to_owned(), "--soul-dir".to_owned()]).is_err());
        assert!(parse_args(["demo".to_owned(), "--soul-dir".to_owned()]).is_err());
        assert!(
            parse_args([
                "serve".to_owned(),
                "--soul-dir".to_owned(),
                "/tmp/a".to_owned(),
                "--soul-dir".to_owned(),
                "/tmp/b".to_owned()
            ])
            .is_err()
        );
        assert!(parse_args(["serve".to_owned(), "--verbose".to_owned()]).is_err());
        let bare = parse_args(["ctl".to_owned()]).expect_err("bare ctl");
        assert!(bare.contains("wake"), "{bare}");
        assert!(parse_args(["ctl".to_owned(), "status".to_owned(), "sleep".to_owned()]).is_err());
        assert_eq!(
            parse_args(["ctl".to_owned(), "wake".to_owned(), "extra".to_owned()])
                .expect_err("extra"),
            "unknown ctl argument extra"
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "wake".to_owned(), "resume".to_owned()])
                .expect_err("two commands"),
            "ctl takes one command"
        );
        assert!(parse_args(["ctl".to_owned(), "tool".to_owned()]).is_err());
        assert!(parse_args(["ctl".to_owned(), "confirm-tool".to_owned()]).is_err());
        assert!(parse_args(["ctl".to_owned(), "cancel-tool".to_owned()]).is_err());
        assert_eq!(
            parse_args([
                "ctl".to_owned(),
                "confirm-tool".to_owned(),
                "1".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned()
            ]),
            Ok(Mode::Ctl {
                socket: Some(PathBuf::from("/tmp/sw.sock")),
                command: CtlAction::ConfirmTool {
                    pending_id: "1".to_owned(),
                }
            })
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "cancel-tool".to_owned(), "4".to_owned()]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::CancelTool {
                    pending_id: "4".to_owned(),
                }
            })
        );
        assert_eq!(
            parse_args([
                "ctl".to_owned(),
                "tool".to_owned(),
                "Echo".to_owned(),
                "Hello".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned(),
                "world".to_owned()
            ]),
            Ok(Mode::Ctl {
                socket: Some(PathBuf::from("/tmp/sw.sock")),
                command: CtlAction::Tool {
                    name: "echo".to_owned(),
                    args: vec!["Hello".to_owned(), "world".to_owned()],
                }
            })
        );
        assert!(
            parse_args([
                "ctl".to_owned(),
                "--socket".to_owned(),
                "/tmp/a".to_owned(),
                "--socket".to_owned(),
                "/tmp/b".to_owned(),
                "status".to_owned()
            ])
            .is_err()
        );
    }

    #[test]
    fn ctl_ask_and_chat_join_words_and_reject_blank_text() {
        assert_eq!(
            parse_args(["ctl".to_owned(), "ask".to_owned()]).expect_err("blank"),
            "ctl ask needs text"
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "chat".to_owned(), "   ".to_owned()]).expect_err("blank"),
            "ctl chat needs text"
        );
        assert_eq!(
            parse_args([
                "ctl".to_owned(),
                "ask".to_owned(),
                "hello".to_owned(),
                "there".to_owned()
            ]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::Ask {
                    text: "hello there".to_owned(),
                }
            })
        );
        assert_eq!(
            parse_args([
                "ctl".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned(),
                "chat".to_owned(),
                "hello".to_owned()
            ]),
            Ok(Mode::Ctl {
                socket: Some(PathBuf::from("/tmp/sw.sock")),
                command: CtlAction::Chat {
                    text: "hello".to_owned(),
                }
            })
        );
    }

    #[test]
    fn strip_line_ending_matches_bufread_lines() {
        let mut newline = "wake\n".to_owned();
        strip_line_ending(&mut newline);
        assert_eq!(newline, "wake");

        let mut crlf = "sleep\r\n".to_owned();
        strip_line_ending(&mut crlf);
        assert_eq!(crlf, "sleep");

        let mut bare = "quit".to_owned();
        strip_line_ending(&mut bare);
        assert_eq!(bare, "quit");

        let mut carriage = "wake\r".to_owned();
        strip_line_ending(&mut carriage);
        assert_eq!(carriage, "wake\r");
    }
}
