//! Disk-only Softwake ctl effects (no voice-state machine). Shared by Runtime and Demo.

use softwake_tools::SoftwakeCtlEffect;

/// Apply ctl effects that only touch Settings / providers / profiles on disk.
///
/// Returns `None` when the effect needs Runtime (sleep, refresh, status meter, …).
pub(crate) fn try_apply_disk_ctl(effect: &SoftwakeCtlEffect) -> Option<Result<String, String>> {
    match effect {
        SoftwakeCtlEffect::ListModels => Some(crate::slash::format_model_list()),
        SoftwakeCtlEffect::ListVoices => Some(crate::slash::format_voice_list()),
        SoftwakeCtlEffect::ListProfiles => Some(crate::slash::format_profile_list()),
        SoftwakeCtlEffect::ListReasoning => Some(crate::slash::format_reasoning_list()),
        SoftwakeCtlEffect::SetModel { which, id } => Some(if which == "voice" {
            crate::slash::set_voice_model(id)
        } else {
            crate::slash::set_chat_model(id)
        }),
        SoftwakeCtlEffect::SetVoice { id } => Some(crate::slash::set_tts_voice(id)),
        SoftwakeCtlEffect::SetReasoning { mode } => Some(crate::slash::set_reasoning_effort(mode)),
        SoftwakeCtlEffect::Status
        | SoftwakeCtlEffect::SetProfile { .. }
        | SoftwakeCtlEffect::Sleep
        | SoftwakeCtlEffect::Hibernate
        | SoftwakeCtlEffect::Resume
        | SoftwakeCtlEffect::NewSession
        | SoftwakeCtlEffect::Refresh => None,
    }
}
