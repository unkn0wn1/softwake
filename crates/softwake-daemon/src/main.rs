//! Softwake daemon entry point.
//!
//! With no arguments the process prints the initial voice state and exits.
//! `demo` reads commands from stdin and walks sleep, awake, and hibernate
//! through mock capture and the text phrase detector.

mod demo;

use std::env;
use std::io::{self, BufRead};
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
        Ok(Mode::Demo) => run_demo(),
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

fn run_demo() -> ExitCode {
    let mut demo = Demo::new(PhraseTable::default(), CooldownConfig::default());
    for line in demo.banner_lines() {
        println!("{line}");
    }
    let mut last = Instant::now();
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("softwaked: stdin: {error}");
                return ExitCode::from(1);
            }
        };
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(last);
        last = now;
        let result = demo.handle_line(&line, elapsed);
        for output in result.lines {
            println!("{output}");
        }
        if result.quit {
            return ExitCode::SUCCESS;
        }
    }
    ExitCode::SUCCESS
}

fn print_help() {
    println!(
        "\
softwaked — Softwake daemon

Usage:
  softwaked            print the initial voice state and exit
  softwaked demo       interactive voice-state demo
  softwaked --demo     same as demo
  softwaked --help     print this help

Demo commands, one per line:
  wake, sleep, hibernate, resume, status, quit"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Status,
    Demo,
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
        Some("demo" | "--demo") => match args.next() {
            None => Ok(Mode::Demo),
            Some(extra) => Err(format!("demo does not take arguments (got {extra})")),
        },
        Some("help" | "--help" | "-h") => match args.next() {
            None => Ok(Mode::Help),
            Some(extra) => Err(format!("help does not take arguments (got {extra})")),
        },
        Some(other) => Err(format!("unknown argument {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, parse_args, status_line};

    #[test]
    fn no_args_is_the_status_mode() {
        assert_eq!(parse_args(std::iter::empty::<String>()), Ok(Mode::Status));
        assert_eq!(status_line(), "softwaked state: sleep");
    }

    #[test]
    fn demo_and_help_flags() {
        assert_eq!(parse_args(["demo".to_owned()]), Ok(Mode::Demo));
        assert_eq!(parse_args(["--demo".to_owned()]), Ok(Mode::Demo));
        assert_eq!(parse_args(["--help".to_owned()]), Ok(Mode::Help));
        assert_eq!(parse_args(["-h".to_owned()]), Ok(Mode::Help));
    }

    #[test]
    fn unknown_and_extra_arguments_are_errors() {
        assert!(parse_args(["--nope".to_owned()]).is_err());
        assert!(parse_args(["demo".to_owned(), "--extra".to_owned()]).is_err());
    }
}
