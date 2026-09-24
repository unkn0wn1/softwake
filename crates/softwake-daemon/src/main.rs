//! Softwake daemon entry point.
//!
//! With no arguments the process prints the initial voice state and exits.
//! `demo` reads typed commands from stdin and walks sleep, awake, and
//! hibernate through mock capture and the text phrase detector. The
//! microphone and `PipeWire` are not wired yet. `--verbose`, `-v`, or
//! `SOFTWAKE_LOG=debug` adds per-line processing detail.

mod demo;

use std::env;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::time::Instant;

use softwake_state::{CooldownConfig, Machine};
use softwake_wake::PhraseTable;

use demo::Demo;

fn main() -> ExitCode {
    match parse_args(env::args().skip(1)) {
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
        Err(message) => {
            eprintln!("softwaked: {message}");
            print_help();
            ExitCode::from(2)
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
  softwaked --help          print this help

The demo reads typed commands only and prints "> " before each line.
Mic and PipeWire are not wired yet.
SOFTWAKE_LOG=debug enables the same detail as --verbose and -v.

Demo commands, one per line:
  wake, sleep, hibernate, resume, status, quit"#
}

fn print_help() {
    println!("{}", help_text());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Status,
    Demo { verbose: bool },
    Help,
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

#[cfg(test)]
mod tests {
    use super::{Mode, debug_log_requested, help_text, parse_args, status_line, strip_line_ending};

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
