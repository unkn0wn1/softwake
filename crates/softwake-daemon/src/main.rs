//! Softwake daemon entry point.
//!
//! The process reports the default voice state and exits. Capture, wake,
//! IPC, and tools are not wired yet.

use softwake_state::{CooldownConfig, Machine};

fn main() {
    let machine = Machine::new(CooldownConfig::default());
    let state = machine.state();
    println!("softwaked state: {state}");
}
