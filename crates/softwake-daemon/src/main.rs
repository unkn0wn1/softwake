//! Process entry. Argument handling and the serve loop live in the library
//! so tests can drive them without spawning this binary.

fn main() -> std::process::ExitCode {
    softwake_daemon::execute(std::env::args().skip(1))
}
