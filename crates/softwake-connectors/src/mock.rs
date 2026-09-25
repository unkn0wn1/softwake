//! In-memory email backend for tests.
//!
//! [`MockEmail::send`] appends one message to an outbox on this value.
//! It does not open a socket, read credentials, or share state with another
//! [`MockEmail`].

use crate::{EmailConnector, OutboundEmail, SendReceipt};

/// Mailbox stand-in. The outbox starts empty.
///
/// Send ids start at 1 for each value. Two mocks do not share a counter.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `MockEmail` is the public name of this backend.
pub struct MockEmail {
    next_id: u64,
    outbox: Vec<OutboundEmail>,
}

impl MockEmail {
    /// Messages accepted by [`MockEmail::send`], oldest first.
    #[must_use]
    pub fn outbox(&self) -> &[OutboundEmail] {
        &self.outbox
    }

    /// Append `message` and return its id.
    ///
    /// The id is `1` for the first send on this value and increases by 1.
    /// The stored message is a clone of `message`, including empty fields.
    #[must_use]
    pub fn send(&mut self, message: &OutboundEmail) -> SendReceipt {
        self.next_id += 1;
        self.outbox.push(message.clone());
        SendReceipt { id: self.next_id }
    }
}

impl EmailConnector for MockEmail {
    type Error = std::convert::Infallible;

    fn send(&mut self, message: &OutboundEmail) -> Result<SendReceipt, Self::Error> {
        Ok(MockEmail::send(self, message))
    }
}

#[cfg(test)]
mod tests {
    use super::MockEmail;
    use crate::OutboundEmail;

    fn message(to: &str, subject: &str, body: &str) -> OutboundEmail {
        OutboundEmail {
            to: to.to_owned(),
            subject: subject.to_owned(),
            body: body.to_owned(),
        }
    }

    #[test]
    fn send_appends_in_order_and_ids_start_at_one() {
        let mut email = MockEmail::default();
        let first = message("a@example.com", "one", "1");
        let second = message("b@example.com", "two", "2");
        assert_eq!(email.send(&first).id, 1);
        assert_eq!(email.send(&second).id, 2);
        assert_eq!(email.outbox(), &[first, second]);
    }

    #[test]
    fn send_stores_fields_unchanged() {
        let mut email = MockEmail::default();
        let stored = message("", "café", "line");
        assert_eq!(email.send(&stored).id, 1);
        assert_eq!(email.outbox(), &[stored]);
    }

    #[test]
    fn two_mocks_do_not_share_an_outbox() {
        let mut first = MockEmail::default();
        let mut second = MockEmail::default();
        let stored = message("a@example.com", "one", "1");
        assert_eq!(first.send(&stored).id, 1);
        assert!(second.outbox().is_empty());
        assert_eq!(second.send(&stored).id, 1);
        assert_eq!(first.outbox(), std::slice::from_ref(&stored));
        assert_eq!(second.outbox(), std::slice::from_ref(&stored));
    }
}
