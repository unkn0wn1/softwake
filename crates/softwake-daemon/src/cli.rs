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
use softwake_state::{CooldownConfig, Machine};
use softwake_wake::PhraseTable;

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
        Ok(Mode::Demo { verbose }) => {
            let verbose = verbose || debug_log_requested(env::var("SOFTWAKE_LOG").ok().as_deref());
            run_demo(verbose)
        }
        Ok(Mode::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Mode::Serve { socket }) => match serve::run(socket.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("softwaked: {error}");
                ExitCode::from(1)
            }
        },
        Ok(Mode::Ctl { socket, command }) => run_ctl(socket.as_deref(), command),
        Err(message) => {
            eprintln!("softwaked: {message}");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn run_ctl(socket: Option<&Path>, command: CtlAction) -> ExitCode {
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

fn run_demo(verbose: bool) -> ExitCode {
    let mut demo =
        Demo::new(PhraseTable::default(), CooldownConfig::default()).with_verbose(verbose);
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
  softwaked serve           listen for ctl and UI clients
  softwaked --serve         same as serve
  softwaked serve --socket PATH
  softwaked ctl status      print the running daemon's voice state
  softwaked ctl hibernate   hibernate (stop capture)
  softwaked ctl resume      leave hibernate and land in sleep
  softwaked ctl sleep       sleep from awake
  softwaked ctl reload-soul record a soul reload for the next awake session
  softwaked --help          print this help

The demo reads typed commands only and prints "> " before each line.
Mic and PipeWire are not wired yet.
SOFTWAKE_LOG=debug enables the same detail as --verbose and -v.

Demo commands, one per line:
  wake, sleep, hibernate, resume, status, quit

Serve owns the voice-state machine and mock capture. It speaks
newline-delimited JSON (protocol 1) on a Unix socket. A stale socket file
is removed on startup. If another serve is already listening, startup
fails and leaves that socket in place.

Socket path, first match wins:
  --socket PATH
  SOFTWAKE_SOCKET
  $XDG_RUNTIME_DIR/softwake/softwaked.sock
  /tmp/softwake-$UID/softwaked.sock when XDG_RUNTIME_DIR is unset

ctl exits non-zero when the daemon rejects a command or cannot be reached.
reload-soul does not parse the soul pack; it records that a reload should
apply on the next awake session."#
}

fn print_help() {
    println!("{}", help_text());
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Status,
    Demo {
        verbose: bool,
    },
    Help,
    Serve {
        socket: Option<PathBuf>,
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
    for arg in args {
        match arg.as_str() {
            "--verbose" | "-v" => verbose = true,
            other => return Err(format!("unknown demo argument {other}")),
        }
    }
    Ok(Mode::Demo { verbose })
}

fn parse_serve_flags(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let mut socket = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => take_socket("serve", &mut args, &mut socket)?,
            other => return Err(format!("unknown serve argument {other}")),
        }
    }
    Ok(Mode::Serve { socket })
}

fn parse_ctl(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let mut socket = None;
    let mut command = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => take_socket("ctl", &mut args, &mut socket)?,
            other => {
                let Some(parsed) = CtlAction::parse(other) else {
                    return Err(format!("unknown ctl argument {other}"));
                };
                if command.is_some() {
                    return Err("ctl takes one command".to_owned());
                }
                command = Some(parsed);
            }
        }
    }
    let Some(command) = command else {
        return Err(
            "ctl needs a command: status, hibernate, resume, sleep, reload-soul".to_owned(),
        );
    };
    Ok(Mode::Ctl { socket, command })
}

fn take_socket(
    mode: &str,
    args: &mut impl Iterator<Item = String>,
    slot: &mut Option<PathBuf>,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{mode} accepts one --socket"));
    }
    let Some(path) = args.next() else {
        return Err("--socket needs a path".to_owned());
    };
    if path.is_empty() {
        return Err("--socket needs a path".to_owned());
    }
    *slot = Some(PathBuf::from(path));
    Ok(())
}

#[cfg(test)]
mod tests {
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
            Ok(Mode::Demo { verbose: false })
        );
        assert_eq!(
            parse_args(["--demo".to_owned()]),
            Ok(Mode::Demo { verbose: false })
        );
        assert_eq!(parse_args(["--help".to_owned()]), Ok(Mode::Help));
        assert_eq!(parse_args(["-h".to_owned()]), Ok(Mode::Help));
    }

    #[test]
    fn demo_verbose_flags() {
        assert_eq!(
            parse_args(["demo".to_owned(), "--verbose".to_owned()]),
            Ok(Mode::Demo { verbose: true })
        );
        assert_eq!(
            parse_args(["demo".to_owned(), "-v".to_owned()]),
            Ok(Mode::Demo { verbose: true })
        );
        assert_eq!(
            parse_args(["--demo".to_owned(), "-v".to_owned()]),
            Ok(Mode::Demo { verbose: true })
        );
        assert_eq!(
            parse_args(["demo".to_owned(), "-v".to_owned(), "--verbose".to_owned()]),
            Ok(Mode::Demo { verbose: true })
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
        assert!(help.contains("reload-soul"));
        assert!(help.contains("SOFTWAKE_SOCKET"));
        assert!(help.contains("XDG_RUNTIME_DIR"));
        assert!(help.contains("/tmp/softwake-$UID/softwaked.sock"));
        assert!(help.contains("stale socket"));
        assert!(help.contains("newline-delimited JSON"));
    }

    #[test]
    fn serve_and_ctl_flags() {
        assert_eq!(
            parse_args(["serve".to_owned()]),
            Ok(Mode::Serve { socket: None })
        );
        assert_eq!(
            parse_args([
                "--serve".to_owned(),
                "--socket".to_owned(),
                "/tmp/sw.sock".to_owned()
            ]),
            Ok(Mode::Serve {
                socket: Some(PathBuf::from("/tmp/sw.sock"))
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
        assert!(parse_args(["serve".to_owned(), "--verbose".to_owned()]).is_err());
        assert!(parse_args(["ctl".to_owned()]).is_err());
        assert!(parse_args(["ctl".to_owned(), "status".to_owned(), "sleep".to_owned()]).is_err());
        assert!(parse_args(["ctl".to_owned(), "wake".to_owned()]).is_err());
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
