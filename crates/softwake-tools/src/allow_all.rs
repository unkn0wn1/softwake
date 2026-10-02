//! Profile `allow_all` grant rewrite (ADR-0052).

use crate::SOFTWARE_INSTALL_TOOL;
use crate::settings::ToolPermission;

/// Apply profile `allow_all` to an operator grant.
///
/// Registry Deny stays Deny (callers must not pass Deny floors here as Always).
/// `software_install` is never auto-lifted by `allow_all`.
#[must_use]
pub fn grant_with_allow_all(
    allow_all: bool,
    tool_name: &str,
    grant: ToolPermission,
) -> ToolPermission {
    if !allow_all {
        return grant;
    }
    if tool_name == SOFTWARE_INSTALL_TOOL {
        return grant;
    }
    match grant {
        ToolPermission::Ask => ToolPermission::AlwaysAllow,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all_lifts_ask_except_install() {
        assert_eq!(
            grant_with_allow_all(true, "shell", ToolPermission::Ask),
            ToolPermission::AlwaysAllow
        );
        assert_eq!(
            grant_with_allow_all(true, SOFTWARE_INSTALL_TOOL, ToolPermission::Ask),
            ToolPermission::Ask
        );
        assert_eq!(
            grant_with_allow_all(true, "shell", ToolPermission::Deny),
            ToolPermission::Deny
        );
        assert_eq!(
            grant_with_allow_all(false, "shell", ToolPermission::Ask),
            ToolPermission::Ask
        );
    }
}
