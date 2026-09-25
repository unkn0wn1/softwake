//! Opt-in live email backend shape.
//!
//! Default Softwake stays on [`crate::MockEmail`]. This type is the scaffold for
//! a later SMTP or provider client: it validates config, stores drafts when
//! mode is draft-only, and refuses real send until a transport is wired.
//! It does not open a socket and does not read the secret bag itself.

use crate::{EmailConnector, EmailSettings, LiveEmailMode, OutboundEmail, SendReceipt};

/// Fixed sentence when live email is off.
pub const LIVE_EMAIL_DISABLED: &str = "live email is off; Softwake uses the in-memory mock outbox";

/// Fixed sentence when SMTP fields are incomplete.
pub const LIVE_EMAIL_NOT_CONFIGURED: &str =
    "live email is not configured: set SMTP host, username, from address, and password";

/// Fixed sentence when mode is send but no transport exists in this build.
pub const LIVE_EMAIL_TRANSPORT_NOT_WIRED: &str =
    "SMTP send is not wired in this build; use draft-only mode or a later transport";

/// Fixed sentence when Test passes for draft-only.
pub const LIVE_EMAIL_TEST_DRAFT_OK: &str =
    "Configuration looks complete. Draft-only mode will store messages locally after confirm.";

/// Fixed sentence when Test passes for send mode (still no socket).
pub const LIVE_EMAIL_TEST_SEND_SCAFFOLD: &str =
    "Configuration looks complete. Send mode still refuses real SMTP in this scaffold.";

/// Why a live backend rejected a send or Test.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LiveEmailError {
    /// Settings toggle is off.
    #[error("{LIVE_EMAIL_DISABLED}")]
    NotEnabled,
    /// Host, username, from, or password is missing.
    #[error("{LIVE_EMAIL_NOT_CONFIGURED}")]
    NotConfigured,
    /// Operator asked for send, but this build has no SMTP client.
    #[error("{LIVE_EMAIL_TRANSPORT_NOT_WIRED}")]
    TransportNotWired,
}

/// Live email handle. Holds non-secret config and whether a password is present.
///
/// The password string stays in the secret bag. This value only knows a bool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEmail {
    settings: EmailSettings,
    password_present: bool,
    next_id: u64,
    drafts: Vec<OutboundEmail>,
}

impl LiveEmail {
    /// Build from Settings and whether the secret bag has an SMTP password.
    #[must_use]
    pub fn new(settings: EmailSettings, password_present: bool) -> Self {
        Self {
            settings,
            password_present,
            next_id: 0,
            drafts: Vec::new(),
        }
    }

    /// Non-secret Settings this value was built from.
    #[must_use]
    pub fn settings(&self) -> &EmailSettings {
        &self.settings
    }

    /// Whether a password was reported present when this value was built.
    #[must_use]
    pub fn password_present(&self) -> bool {
        self.password_present
    }

    /// Drafts accepted in draft-only mode, oldest first.
    #[must_use]
    pub fn drafts(&self) -> &[OutboundEmail] {
        &self.drafts
    }

    /// Mode from Settings.
    #[must_use]
    pub fn mode(&self) -> LiveEmailMode {
        self.settings.mode
    }

    /// Whether host, port, username, from, and password look set.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.settings.is_smtp_complete() && self.password_present
    }

    /// Validate config without opening a socket.
    ///
    /// # Errors
    ///
    /// [`LiveEmailError::NotEnabled`] when the toggle is off.
    /// [`LiveEmailError::NotConfigured`] when SMTP fields or password are missing.
    pub fn test_connection(&self) -> Result<&'static str, LiveEmailError> {
        if !self.settings.live_enabled {
            return Err(LiveEmailError::NotEnabled);
        }
        if !self.is_configured() {
            return Err(LiveEmailError::NotConfigured);
        }
        Ok(match self.settings.mode {
            LiveEmailMode::DraftOnly => LIVE_EMAIL_TEST_DRAFT_OK,
            LiveEmailMode::Send => LIVE_EMAIL_TEST_SEND_SCAFFOLD,
        })
    }
}

impl EmailConnector for LiveEmail {
    type Error = LiveEmailError;

    fn send(&mut self, message: &OutboundEmail) -> Result<SendReceipt, Self::Error> {
        if !self.settings.live_enabled {
            return Err(LiveEmailError::NotEnabled);
        }
        if !self.is_configured() {
            return Err(LiveEmailError::NotConfigured);
        }
        match self.settings.mode {
            LiveEmailMode::DraftOnly => {
                self.next_id += 1;
                self.drafts.push(message.clone());
                Ok(SendReceipt { id: self.next_id })
            }
            LiveEmailMode::Send => Err(LiveEmailError::TransportNotWired),
        }
    }
}

/// Mock or live backend held by the daemon.
///
/// [`EmailBackend::Mock`] is the default and the only path CI exercises for
/// end-to-end sends. [`EmailBackend::Live`] is opt-in via Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmailBackend {
    /// In-memory outbox. No credentials. No socket.
    Mock(crate::MockEmail),
    /// Opt-in live scaffold. Draft-only or not-wired send.
    Live(LiveEmail),
}

impl Default for EmailBackend {
    fn default() -> Self {
        Self::Mock(crate::MockEmail::default())
    }
}

impl EmailBackend {
    /// Build from on-disk Settings and whether an SMTP password is saved.
    ///
    /// When `live_enabled` is false, returns [`EmailBackend::Mock`] regardless
    /// of the other fields.
    #[must_use]
    pub fn from_settings(settings: EmailSettings, password_present: bool) -> Self {
        if settings.live_enabled {
            Self::Live(LiveEmail::new(settings, password_present))
        } else {
            Self::Mock(crate::MockEmail::default())
        }
    }

    /// Messages stored on this backend (mock outbox or live drafts).
    #[must_use]
    pub fn messages(&self) -> &[OutboundEmail] {
        match self {
            Self::Mock(email) => email.outbox(),
            Self::Live(email) => email.drafts(),
        }
    }

    /// `true` when this value is the live scaffold.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live(_))
    }

    /// Live mode when live, otherwise `None`.
    #[must_use]
    pub fn live_mode(&self) -> Option<LiveEmailMode> {
        match self {
            Self::Mock(_) => None,
            Self::Live(email) => Some(email.mode()),
        }
    }

    /// Authorize was already checked. Perform the backend send or draft.
    ///
    /// # Errors
    ///
    /// Live backend configuration or transport errors.
    pub fn send(&mut self, message: &OutboundEmail) -> Result<SendReceipt, LiveEmailError> {
        match self {
            Self::Mock(email) => match EmailConnector::send(email, message) {
                Ok(receipt) => Ok(receipt),
                Err(error) => match error {},
            },
            Self::Live(email) => EmailConnector::send(email, message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EmailBackend, LIVE_EMAIL_NOT_CONFIGURED, LIVE_EMAIL_TEST_DRAFT_OK,
        LIVE_EMAIL_TRANSPORT_NOT_WIRED, LiveEmail, LiveEmailError,
    };
    use crate::{EmailConnector, EmailSettings, LiveEmailMode, OutboundEmail};

    fn message() -> OutboundEmail {
        OutboundEmail {
            to: "ada@example.com".to_owned(),
            subject: "hello".to_owned(),
            body: "note".to_owned(),
        }
    }

    fn complete_settings(mode: LiveEmailMode) -> EmailSettings {
        EmailSettings {
            live_enabled: true,
            smtp_host: "smtp.example.com".to_owned(),
            smtp_port: 587,
            username: "ada".to_owned(),
            from_address: "ada@example.com".to_owned(),
            mode,
            ..EmailSettings::default()
        }
    }

    #[test]
    fn default_backend_is_mock() {
        let backend = EmailBackend::default();
        assert!(!backend.is_live());
        assert!(backend.messages().is_empty());
    }

    #[test]
    fn from_settings_stays_mock_when_live_off() {
        let settings = EmailSettings {
            live_enabled: false,
            smtp_host: "smtp.example.com".to_owned(),
            ..EmailSettings::default()
        };
        let backend = EmailBackend::from_settings(settings, true);
        assert!(!backend.is_live());
    }

    #[test]
    fn test_connection_requires_enable_and_config() {
        let disabled = LiveEmail::new(EmailSettings::default(), false);
        assert_eq!(disabled.test_connection(), Err(LiveEmailError::NotEnabled));

        let incomplete = LiveEmail::new(
            EmailSettings {
                live_enabled: true,
                ..EmailSettings::default()
            },
            false,
        );
        assert_eq!(
            incomplete.test_connection(),
            Err(LiveEmailError::NotConfigured)
        );
        assert_eq!(
            incomplete.test_connection().unwrap_err().to_string(),
            LIVE_EMAIL_NOT_CONFIGURED
        );

        let ready = LiveEmail::new(complete_settings(LiveEmailMode::DraftOnly), true);
        assert_eq!(ready.test_connection(), Ok(LIVE_EMAIL_TEST_DRAFT_OK));
    }

    #[test]
    fn draft_only_appends_after_send() {
        let mut email = LiveEmail::new(complete_settings(LiveEmailMode::DraftOnly), true);
        let stored = message();
        let receipt = EmailConnector::send(&mut email, &stored).expect("draft");
        assert_eq!(receipt.id, 1);
        assert_eq!(email.drafts(), std::slice::from_ref(&stored));
    }

    #[test]
    fn send_mode_refuses_transport() {
        let mut email = LiveEmail::new(complete_settings(LiveEmailMode::Send), true);
        let err = EmailConnector::send(&mut email, &message()).expect_err("no smtp");
        assert_eq!(err, LiveEmailError::TransportNotWired);
        assert_eq!(err.to_string(), LIVE_EMAIL_TRANSPORT_NOT_WIRED);
        assert!(email.drafts().is_empty());
    }

    #[test]
    fn backend_send_mock_and_live() {
        let mut mock = EmailBackend::default();
        let stored = message();
        assert_eq!(mock.send(&stored).expect("mock").id, 1);
        assert_eq!(mock.messages(), std::slice::from_ref(&stored));

        let mut live =
            EmailBackend::from_settings(complete_settings(LiveEmailMode::DraftOnly), true);
        assert_eq!(live.send(&stored).expect("draft").id, 1);
        assert_eq!(live.messages(), std::slice::from_ref(&stored));
        assert_eq!(live.live_mode(), Some(LiveEmailMode::DraftOnly));
    }
}
