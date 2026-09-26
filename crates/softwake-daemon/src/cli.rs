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
            verbosity,
            voice_test,
        }) => {
            let dir = match resolve_soul_dir(soul_dir.as_deref()) {
                Ok(dir) => dir,
                Err(error) => {
                    eprintln!("softwaked: {error}");
                    return ExitCode::from(1);
                }
            };
            let verbosity =
                merge_serve_verbosity(verbosity, env::var("SOFTWAKE_LOG").ok().as_deref());
            match serve::run(socket.as_deref(), dir, capture, verbosity, voice_test) {
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

#[allow(
    clippy::too_many_lines,
    reason = "operator usage text is one string the help test matches"
)]
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
  softwaked serve -v        KWS hear/match logs (keyword + wake/sleep/hibernate) and context size
  softwaked serve -vv       also mic-energy lines while sleeping
  softwaked serve --voice-test
                            phrase transitions and state voice; mic speech is not sent to chat
  softwaked -vv serve       same leading verbosity before serve
  softwaked serve --socket PATH
  softwaked serve --soul-dir PATH
  softwaked serve --capture mock|pipewire
  softwaked ctl status      print the running daemon's voice state
  softwaked ctl hibernate   hibernate (stop capture)
  softwaked ctl resume      leave hibernate and land in sleep
  softwaked ctl wake        enter awake from sleep (valid soul pack required)
  softwaked ctl sleep       sleep from awake
  softwaked ctl reload-soul re-read the soul pack; it applies on the next awake
  softwaked ctl reload-kws  rebuild the keyword spotter from softwake.json thresholds
  softwaked ctl reload-utterance
                            re-read free-speech end silence without rebuilding KWS
  softwaked ctl voice-test [on|off]
                            show or set voice test mode (default off; not saved)
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
SOFTWAKE_LOG=debug enables the same detail as --verbose and -v (demo, or serve -v).
SOFTWAKE_LOG=trace matches serve -vv for KWS mic-energy lines.

Demo commands, one per line:
  wake, sleep, hibernate, resume, status, reload-soul, reload-kws, tool, confirm, cancel, hear, say, ask, chat, quit

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

Voice phrases add bare "hi" (wake), bare "sleep" (sleep), and "deep sleep"
(hibernate). Existing profile and product phrases stay. "deep sleep" stops
listening. Only ctl resume or the UI Resume button leaves hibernate, and that
lands in sleep. ctl wake still enters awake from sleep when the soul pack is
valid. --voice-test keeps those phrase transitions and the short state voice,
and does not send microphone speech to the chat model. Typed ctl ask still
works. The flag clears when serve exits.

At -v, KWS lines include profile=<name> (and id=<id> when they differ) and
context lines show sent / before_compact sizes. A compact line shows before,
after, and the real compact threshold.

Serve owns the voice-state machine and capture. Default capture is mock.
`--capture pipewire` (or SOFTWAKE_CAPTURE=pipewire) opens the default
microphone when the binary was built with `--features pipewire-capture`
(Linux). `--capture wasapi` selects the WASAPI stub when built with
`wasapi-capture` (Windows; no device yet).
It speaks newline-delimited JSON (protocol 1) on local IPC. A stale socket
file (or Windows port file) is removed on startup. If another serve is already listening, startup
fails and leaves that endpoint in place.

IPC path, first match wins (Linux Unix socket / Windows port file):
  --socket PATH
  SOFTWAKE_SOCKET
  Linux: $XDG_RUNTIME_DIR/softwake/softwaked.sock
  Linux: /tmp/softwake-$UID/softwaked.sock when XDG_RUNTIME_DIR is unset
  Windows: %LOCALAPPDATA%\softwake\softwaked.port

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
does not enter awake. Serve wakes from the microphone when built with `sherpa-kws` and KWS weights are installed (else NullDetector).
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
        /// `0` quiet, `1` (`-v`), `2` (`-vv`).
        verbosity: u8,
        /// Phrase lab: no microphone speech is sent to chat. Default off.
        voice_test: bool,
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
    let mut args = args.into_iter().peekable();
    let mut leading_verbosity = 0_u8;
    while let Some(arg) = args.peek() {
        match arg.as_str() {
            "-v" | "--verbose" => {
                leading_verbosity = leading_verbosity.saturating_add(1).min(2);
                args.next();
            }
            "-vv" => {
                leading_verbosity = 2;
                args.next();
            }
            _ => break,
        }
    }
    let first = args.next();
    match first.as_deref() {
        None if leading_verbosity > 0 => {
            Err("verbosity flags need a command (try: softwaked serve -vv)".to_owned())
        }
        None => Ok(Mode::Status),
        Some("demo" | "--demo") => parse_demo_flags(args, leading_verbosity > 0),
        Some("help" | "--help" | "-h") => match args.next() {
            None => Ok(Mode::Help),
            Some(extra) => Err(format!("help does not take arguments (got {extra})")),
        },
        Some("serve" | "--serve") => parse_serve_flags(args, leading_verbosity),
        Some("ctl") => {
            if leading_verbosity > 0 {
                return Err("ctl does not take -v / -vv".to_owned());
            }
            parse_ctl(args)
        }
        Some(other) => Err(format!("unknown argument {other}")),
    }
}

fn parse_demo_flags(
    args: impl IntoIterator<Item = String>,
    leading_verbose: bool,
) -> Result<Mode, String> {
    let mut verbose = leading_verbose;
    let mut soul_dir = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--verbose" | "-v" | "-vv" => verbose = true,
            "--soul-dir" => take_value("demo", "--soul-dir", &mut args, &mut soul_dir)?,
            other => return Err(format!("unknown demo argument {other}")),
        }
    }
    Ok(Mode::Demo { verbose, soul_dir })
}

fn parse_serve_flags(
    args: impl IntoIterator<Item = String>,
    leading_verbosity: u8,
) -> Result<Mode, String> {
    let mut socket = None;
    let mut soul_dir = None;
    let mut capture_flag = None;
    let mut verbosity = leading_verbosity;
    let mut voice_test = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => take_value("serve", "--socket", &mut args, &mut socket)?,
            "--soul-dir" => take_value("serve", "--soul-dir", &mut args, &mut soul_dir)?,
            "--verbose" | "-v" => verbosity = verbosity.saturating_add(1).min(2),
            "-vv" => verbosity = 2,
            "--voice-test" => {
                if voice_test {
                    return Err("serve accepts one --voice-test".to_owned());
                }
                voice_test = true;
            }
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
        verbosity,
        voice_test,
    })
}

/// Merge CLI verbosity with `SOFTWAKE_LOG` (`debug` → 1, `trace` → 2).
fn merge_serve_verbosity(cli: u8, softwake_log: Option<&str>) -> u8 {
    let from_env = match softwake_log.map(str::trim) {
        Some("trace") => 2_u8,
        Some("debug") => 1_u8,
        _ => 0_u8,
    };
    cli.max(from_env)
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
            "ctl needs a command: status, hibernate, resume, wake, sleep, reload-soul, reload-kws, reload-utterance, voice-test, tool, confirm-tool, cancel-tool, ask, chat"
                .to_owned(),
        ),
        [name] if name == "tool" => Err("ctl tool needs a tool name".to_owned()),
        [name] if name == "confirm-tool" || name == "cancel-tool" => {
            Err(format!("ctl {name} needs a pending id"))
        }
        [name] if name == "ask" || name == "chat" => Err(format!("ctl {name} needs text")),
        [name] if name == "voice-test" => Ok(CtlAction::VoiceTest { enabled: None }),
        [name, value] if name == "voice-test" => match value.as_str() {
            "on" => Ok(CtlAction::VoiceTest {
                enabled: Some(true),
            }),
            "off" => Ok(CtlAction::VoiceTest {
                enabled: Some(false),
            }),
            other => Err(format!("ctl voice-test expects on or off (got {other})")),
        },
        [name, _, extra, ..] if name == "voice-test" => {
            Err(format!("ctl voice-test takes one of on or off (got extra {extra})"))
        }
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

    use super::{
        Mode, debug_log_requested, help_text, merge_serve_verbosity, parse_args, status_line,
        strip_line_ending,
    };
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
    fn serve_verbosity_merges_cli_and_env() {
        assert_eq!(merge_serve_verbosity(0, None), 0);
        assert_eq!(merge_serve_verbosity(0, Some("debug")), 1);
        assert_eq!(merge_serve_verbosity(0, Some("trace")), 2);
        assert_eq!(merge_serve_verbosity(2, Some("debug")), 2);
        assert_eq!(merge_serve_verbosity(1, Some("trace")), 2);
    }

    #[test]
    fn help_mentions_typed_only_verbose_and_the_log_env() {
        let help = help_text();
        assert!(help.contains("typed commands only"));
        assert!(help.contains("PipeWire"));
        assert!(help.contains("--verbose"));
        assert!(help.contains("-v"));
        assert!(help.contains("-vv"));
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
                capture: CaptureKind::Mock,
                verbosity: 0,
                voice_test: false,
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
                capture: CaptureKind::Mock,
                verbosity: 0,
                voice_test: false,
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
            parse_args(["ctl".to_owned(), "reload-utterance".to_owned()]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::ReloadUtterance
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
    fn serve_verbosity_flags() {
        assert_eq!(
            parse_args(["serve".to_owned(), "--verbose".to_owned()]),
            Ok(Mode::Serve {
                socket: None,
                soul_dir: None,
                capture: CaptureKind::Mock,
                verbosity: 1,
                voice_test: false,
            })
        );
        assert_eq!(
            parse_args(["serve".to_owned(), "-vv".to_owned()]),
            Ok(Mode::Serve {
                socket: None,
                soul_dir: None,
                capture: CaptureKind::Mock,
                verbosity: 2,
                voice_test: false,
            })
        );
        assert_eq!(
            parse_args(["-vv".to_owned(), "serve".to_owned()]),
            Ok(Mode::Serve {
                socket: None,
                soul_dir: None,
                capture: CaptureKind::Mock,
                verbosity: 2,
                voice_test: false,
            })
        );
        assert_eq!(
            parse_args(["serve".to_owned(), "-v".to_owned(), "-v".to_owned()]),
            Ok(Mode::Serve {
                socket: None,
                soul_dir: None,
                capture: CaptureKind::Mock,
                verbosity: 2,
                voice_test: false,
            })
        );
    }

    #[test]
    fn voice_test_flag_defaults_off_and_ctl_parses_on_off() {
        assert_eq!(
            parse_args(["serve".to_owned(), "--voice-test".to_owned()]),
            Ok(Mode::Serve {
                socket: None,
                soul_dir: None,
                capture: CaptureKind::Mock,
                verbosity: 0,
                voice_test: true,
            })
        );
        assert!(
            parse_args([
                "serve".to_owned(),
                "--voice-test".to_owned(),
                "--voice-test".to_owned()
            ])
            .is_err()
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "voice-test".to_owned()]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::VoiceTest { enabled: None },
            })
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "voice-test".to_owned(), "on".to_owned()]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::VoiceTest {
                    enabled: Some(true),
                },
            })
        );
        assert_eq!(
            parse_args(["ctl".to_owned(), "voice-test".to_owned(), "off".to_owned()]),
            Ok(Mode::Ctl {
                socket: None,
                command: CtlAction::VoiceTest {
                    enabled: Some(false),
                },
            })
        );
        assert!(
            parse_args([
                "ctl".to_owned(),
                "voice-test".to_owned(),
                "maybe".to_owned()
            ])
            .is_err()
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
