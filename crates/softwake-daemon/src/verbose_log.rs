//! Pure formatters for `-v` / `-vv` daemon lines.
//!
//! Profile text is the loaded name (and id when it differs). Callers pass it
//! in. These functions do not read the filesystem and do not invent a name.

/// Fields for one verbose KWS hear line.
pub(crate) struct KwsHeardLine<'a> {
    /// `profile=<name>` or `profile=<name> id=<id>`.
    pub profile: &'a str,
    /// Raw spotter keyword.
    pub keyword: &'a str,
    /// `match=wake` or `match=suppressed reason=long-utterance`.
    pub match_label: &'a str,
    /// Voice state spelling.
    pub state: &'a str,
    /// Peak-normalized RMS of the window.
    pub mic_rms: f32,
    /// Comma-joined wake phrases.
    pub wake: &'a str,
    /// Comma-joined sleep phrases.
    pub sleep: &'a str,
    /// Comma-joined hibernate phrases.
    pub hibernate: &'a str,
}

/// `softwaked: KWS profile=… heard keyword=…`
#[must_use]
pub(crate) fn format_kws_heard(line: &KwsHeardLine<'_>) -> String {
    format!(
        "softwaked: KWS {profile} heard keyword=`{keyword}` {match_label} state={state} mic_rms={mic_rms:.3} wake=[{wake}] sleep=[{sleep}] hibernate=[{hibernate}]",
        profile = line.profile,
        keyword = line.keyword,
        match_label = line.match_label,
        state = line.state,
        mic_rms = line.mic_rms,
        wake = line.wake,
        sleep = line.sleep,
        hibernate = line.hibernate,
    )
}

/// `softwaked: KWS profile=… wake match refused: …`
#[must_use]
pub(crate) fn format_kws_refuse(profile: &str, kind: &str, error: &str) -> String {
    format!("softwaked: KWS {profile} {kind} match refused: {error}")
}

/// `softwaked: KWS profile=… silence reset`
#[must_use]
pub(crate) fn format_kws_silence(profile: &str) -> String {
    format!("softwaked: KWS {profile} silence reset")
}

/// `softwaked: voice profile=… sleep -> awake (wake phrase)`
#[must_use]
pub(crate) fn format_voice_transition(profile: &str, from: &str, to: &str, event: &str) -> String {
    format!("softwaked: voice {profile} {from} -> {to} ({event})")
}

#[cfg(test)]
mod tests {
    use super::{
        KwsHeardLine, format_kws_heard, format_kws_refuse, format_kws_silence,
        format_voice_transition,
    };

    #[test]
    fn kws_and_voice_lines_include_the_loaded_profile() {
        let heard = format_kws_heard(&KwsHeardLine {
            profile: "profile=sally id=default",
            keyword: "hi",
            match_label: "match=wake",
            state: "sleep",
            mic_rms: 0.123,
            wake: "sally, hi",
            sleep: "sleep",
            hibernate: "deep sleep",
        });
        assert_eq!(
            heard,
            "softwaked: KWS profile=sally id=default heard keyword=`hi` match=wake state=sleep mic_rms=0.123 wake=[sally, hi] sleep=[sleep] hibernate=[deep sleep]"
        );
        assert_eq!(
            format_kws_refuse("profile=sally", "hibernate", "soul pack is missing"),
            "softwaked: KWS profile=sally hibernate match refused: soul pack is missing"
        );
        assert_eq!(
            format_kws_silence("profile=sally"),
            "softwaked: KWS profile=sally silence reset"
        );
        assert_eq!(
            format_voice_transition("profile=sally id=default", "sleep", "awake", "wake phrase"),
            "softwaked: voice profile=sally id=default sleep -> awake (wake phrase)"
        );
    }
}
