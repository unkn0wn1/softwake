//! Outbound email types and the [`EmailConnector`] trait.

/// One message a connector may send.
///
/// The strings are stored unchanged. Nothing in this crate parses an address,
/// trims the fields, or treats them as a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundEmail {
    /// Recipient text. Not validated.
    pub to: String,
    /// Subject line. Not validated.
    pub subject: String,
    /// Body text. Not validated.
    pub body: String,
}

/// Identifier for one send on a single connector value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendReceipt {
    /// `1` for the first send on that value, then `2`, and so on.
    pub id: u64,
}

/// Send one email through a backend.
///
/// The registry decides whether a caller may send. This trait only performs
/// the send the caller already chose. It does not open a socket by itself;
/// a backend that talks to a mailbox would, and the default mock does not.
pub trait EmailConnector {
    /// Backend failure while accepting a message.
    type Error: std::error::Error;

    /// Send `message` and return the backend's receipt.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the message cannot be accepted.
    fn send(&mut self, message: &OutboundEmail) -> Result<SendReceipt, Self::Error>;
}
