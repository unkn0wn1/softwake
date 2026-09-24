//! Gate a tool call on the voice state, then on the allowlist.
//!
//! [`softwake_state::Machine::permit_tool_dispatch`] runs first. Sleep and
//! hibernate never reach the registry. An unknown name is rejected only
//! while awake.

use softwake_state::{Machine, StateError, VoiceState};
use softwake_tools::{ToolError, ToolRegistry, ToolResult};

/// Why a tool call did not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DispatchError {
    /// The daemon is not awake.
    #[error("cannot run {name} while {state}")]
    Forbidden {
        /// Tool the caller named.
        name: String,
        /// State that refused the call.
        state: VoiceState,
    },

    /// The name is not on the phase-1 allowlist.
    #[error("unknown tool: {name}")]
    Unknown {
        /// Name that was rejected.
        name: String,
    },
}

/// Run `name` when `machine` is awake and the name is allowlisted.
///
/// # Errors
///
/// Returns [`DispatchError::Forbidden`] when the machine is not awake, or
/// [`DispatchError::Unknown`] when the name is not allowlisted.
pub(crate) fn invoke(
    machine: &Machine,
    name: &str,
    args: &[String],
) -> Result<ToolResult, DispatchError> {
    match machine.permit_tool_dispatch() {
        Ok(()) => {}
        Err(StateError::ToolsForbidden { state }) => {
            return Err(DispatchError::Forbidden {
                name: name.to_owned(),
                state,
            });
        }
        Err(_) => {
            return Err(DispatchError::Forbidden {
                name: name.to_owned(),
                state: machine.state(),
            });
        }
    }
    ToolRegistry::phase1()
        .invoke(name, args)
        .map_err(|error| match error {
            ToolError::Unknown { name } => DispatchError::Unknown { name },
        })
}
