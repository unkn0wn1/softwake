//! Capture backend selection for `softwaked serve`.
//!
//! Default is [`MockAudioCapture`] (CI/demo). Opt into `PipeWire` with
//! `--capture pipewire` or `SOFTWAKE_CAPTURE=pipewire` when the daemon was
//! built with the `pipewire-capture` feature (Linux). Opt into the WASAPI
//! stub with `--capture wasapi` when built with `wasapi-capture` (Windows;
//! does not open a device yet).

use softwake_audio::{AudioCapture, AudioFrame, MockAudioCapture};

#[cfg(feature = "pipewire-capture")]
use softwake_audio::{PipeWireCapture, PipeWireError};
#[cfg(feature = "wasapi-capture")]
use softwake_audio::{WasapiCapture, WasapiError};

/// Which microphone backend `serve` should open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureKind {
    /// In-memory tone / test frames. Default.
    Mock,
    /// Default `PipeWire` input at [`softwake_audio::AudioFormat::WAKE`].
    PipeWire,
    /// WASAPI stub (Windows). Does not open a device yet.
    #[cfg(feature = "wasapi-capture")]
    Wasapi,
}

impl CaptureKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::PipeWire => "pipewire",
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi => "wasapi",
        }
    }
}

/// Parse `--capture` / `SOFTWAKE_CAPTURE` values.
///
/// # Errors
///
/// Unknown names, or `pipewire` when the binary was not built with
/// `pipewire-capture`.
pub(crate) fn parse_capture_kind(raw: &str) -> Result<CaptureKind, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "mock" | "demo" => Ok(CaptureKind::Mock),
        "pipewire" | "pw" | "mic" => {
            #[cfg(feature = "pipewire-capture")]
            {
                Ok(CaptureKind::PipeWire)
            }
            #[cfg(not(feature = "pipewire-capture"))]
            {
                Err(
                    "capture pipewire needs a build with --features pipewire-capture (and libpipewire-0.3-dev)"
                        .to_owned(),
                )
            }
        }
        "wasapi" | "win" => {
            #[cfg(feature = "wasapi-capture")]
            {
                Ok(CaptureKind::Wasapi)
            }
            #[cfg(not(feature = "wasapi-capture"))]
            {
                Err("capture wasapi needs a build with --features wasapi-capture".to_owned())
            }
        }
        other => Err(format!(
            "unknown capture backend {other} (expected mock, pipewire, or wasapi)"
        )),
    }
}

/// Resolve CLI flag, then `SOFTWAKE_CAPTURE`, then mock.
///
/// # Errors
///
/// Returns the parse error when a value is present and invalid.
pub(crate) fn resolve_capture_kind(
    cli: Option<&str>,
    env: Option<&str>,
) -> Result<CaptureKind, String> {
    if let Some(value) = cli {
        return parse_capture_kind(value);
    }
    if let Some(value) = env.filter(|value| !value.trim().is_empty()) {
        return parse_capture_kind(value);
    }
    Ok(CaptureKind::Mock)
}

/// Failure while opening, reading, or releasing the selected backend.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct CaptureError(pub(crate) String);

#[cfg(feature = "pipewire-capture")]
impl From<PipeWireError> for CaptureError {
    fn from(error: PipeWireError) -> Self {
        Self(error.to_string())
    }
}

#[cfg(feature = "wasapi-capture")]
impl From<WasapiError> for CaptureError {
    fn from(error: WasapiError) -> Self {
        Self(error.to_string())
    }
}

/// Mock or (feature-gated) `PipeWire` capture behind one drain API.
#[derive(Debug)]
pub(crate) enum CaptureBackend {
    Mock(MockAudioCapture),
    #[cfg(feature = "pipewire-capture")]
    PipeWire(PipeWireCapture),
    #[cfg(feature = "wasapi-capture")]
    Wasapi(WasapiCapture),
}

impl CaptureBackend {
    #[allow(
        clippy::unnecessary_wraps,
        reason = "PipeWire open returns CaptureError when pipewire-capture is enabled"
    )]
    pub(crate) fn open(kind: CaptureKind) -> Result<Self, CaptureError> {
        match kind {
            CaptureKind::Mock => {
                let mut capture = MockAudioCapture::default();
                let Ok(()) = capture.start();
                Ok(Self::Mock(capture))
            }
            CaptureKind::PipeWire => {
                #[cfg(feature = "pipewire-capture")]
                {
                    let mut capture = PipeWireCapture::new();
                    capture.start()?;
                    Ok(Self::PipeWire(capture))
                }
                #[cfg(not(feature = "pipewire-capture"))]
                {
                    // `parse_capture_kind` rejects this before open.
                    unreachable!("pipewire capture without pipewire-capture feature")
                }
            }
            #[cfg(feature = "wasapi-capture")]
            CaptureKind::Wasapi => {
                let mut capture = WasapiCapture::new();
                capture.start()?;
                Ok(Self::Wasapi(capture))
            }
        }
    }

    pub(crate) const fn kind(&self) -> CaptureKind {
        match self {
            Self::Mock(_) => CaptureKind::Mock,
            #[cfg(feature = "pipewire-capture")]
            Self::PipeWire(_) => CaptureKind::PipeWire,
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi(_) => CaptureKind::Wasapi,
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        match self {
            Self::Mock(capture) => capture.is_running(),
            #[cfg(feature = "pipewire-capture")]
            Self::PipeWire(capture) => capture.is_running(),
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi(capture) => capture.is_running(),
        }
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "PipeWire start returns CaptureError when pipewire-capture is enabled"
    )]
    pub(crate) fn start(&mut self) -> Result<(), CaptureError> {
        match self {
            Self::Mock(capture) => {
                let Ok(()) = capture.start();
                Ok(())
            }
            #[cfg(feature = "pipewire-capture")]
            Self::PipeWire(capture) => {
                capture.start()?;
                Ok(())
            }
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi(capture) => {
                capture.start()?;
                Ok(())
            }
        }
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "PipeWire stop returns CaptureError when pipewire-capture is enabled"
    )]
    pub(crate) fn stop(&mut self) -> Result<(), CaptureError> {
        match self {
            Self::Mock(capture) => {
                let Ok(()) = capture.stop();
                Ok(())
            }
            #[cfg(feature = "pipewire-capture")]
            Self::PipeWire(capture) => {
                capture.stop()?;
                Ok(())
            }
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi(capture) => {
                capture.stop()?;
                Ok(())
            }
        }
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "PipeWire poll returns CaptureError when pipewire-capture is enabled"
    )]
    pub(crate) fn poll_frame(&mut self) -> Result<Option<AudioFrame>, CaptureError> {
        match self {
            Self::Mock(capture) => {
                let Ok(frame) = AudioCapture::poll_frame(capture);
                Ok(frame)
            }
            #[cfg(feature = "pipewire-capture")]
            Self::PipeWire(capture) => Ok(AudioCapture::poll_frame(capture)?),
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi(capture) => Ok(AudioCapture::poll_frame(capture)?),
        }
    }

    /// Queue the mock listening tone. No-op for `PipeWire` (real PCM only).
    pub(crate) fn push_listening_tone_if_mock(&mut self, phase_secs: f32) {
        match self {
            Self::Mock(capture) => {
                let _ = capture.push_listening_tone(phase_secs);
            }
            #[cfg(feature = "pipewire-capture")]
            Self::PipeWire(_) => {}
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi(_) => {}
        }
    }

    /// Mutable mock handle for tests that inject frames.
    #[cfg(test)]
    pub(crate) fn mock_mut(&mut self) -> &mut MockAudioCapture {
        match self {
            Self::Mock(capture) => capture,
            #[cfg(feature = "pipewire-capture")]
            Self::PipeWire(_) => panic!("expected mock capture in tests"),
            #[cfg(feature = "wasapi-capture")]
            Self::Wasapi(_) => panic!("expected mock capture in tests"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureKind, parse_capture_kind, resolve_capture_kind};

    #[test]
    fn mock_is_default_and_aliases_parse() {
        assert_eq!(
            resolve_capture_kind(None, None).expect("default"),
            CaptureKind::Mock
        );
        assert_eq!(parse_capture_kind("mock").expect("mock"), CaptureKind::Mock);
        assert_eq!(parse_capture_kind("DEMO").expect("demo"), CaptureKind::Mock);
    }

    #[test]
    fn unknown_name_is_rejected() {
        let error = parse_capture_kind("jack").expect_err("unknown");
        assert!(error.contains("unknown capture"), "{error}");
    }

    #[cfg(feature = "pipewire-capture")]
    #[test]
    fn pipewire_aliases_parse_when_linked() {
        assert_eq!(
            parse_capture_kind("pipewire").expect("pw"),
            CaptureKind::PipeWire
        );
        assert_eq!(parse_capture_kind("pw").expect("pw"), CaptureKind::PipeWire);
        assert_eq!(
            resolve_capture_kind(None, Some("pipewire")).expect("env"),
            CaptureKind::PipeWire
        );
        assert_eq!(
            resolve_capture_kind(Some("mock"), Some("pipewire")).expect("cli wins"),
            CaptureKind::Mock
        );
    }

    #[cfg(not(feature = "pipewire-capture"))]
    #[test]
    fn pipewire_rejected_without_feature() {
        let error = parse_capture_kind("pipewire").expect_err("needs feature");
        assert!(error.contains("pipewire-capture"), "{error}");
    }

    #[cfg(not(feature = "wasapi-capture"))]
    #[test]
    fn wasapi_rejected_without_feature() {
        let error = parse_capture_kind("wasapi").expect_err("needs feature");
        assert!(error.contains("wasapi-capture"), "{error}");
    }

    #[cfg(feature = "wasapi-capture")]
    #[test]
    fn wasapi_parses_when_linked() {
        assert_eq!(
            parse_capture_kind("wasapi").expect("wasapi"),
            CaptureKind::Wasapi
        );
    }
}
