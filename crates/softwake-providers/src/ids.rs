//! Provider identifiers.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Stable provider id used in Settings and the secret bag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    /// xAI device-code OAuth.
    XaiOauth,
    /// xAI API key.
    XaiKey,
    /// `OpenAI` API key.
    Openai,
}

impl ProviderId {
    /// Wire and Settings spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::XaiOauth => "xai-oauth",
            Self::XaiKey => "xai-key",
            Self::Openai => "openai",
        }
    }

    /// Every registered id, in display order.
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::XaiOauth, Self::XaiKey, Self::Openai]
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = ParseProviderIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "xai-oauth" => Ok(Self::XaiOauth),
            "xai-key" => Ok(Self::XaiKey),
            "openai" => Ok(Self::Openai),
            _ => Err(ParseProviderIdError {
                value: s.to_owned(),
            }),
        }
    }
}

/// Unknown provider id string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown provider id `{value}`")]
pub struct ParseProviderIdError {
    /// Rejected spelling.
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::ProviderId;

    #[test]
    fn round_trip_wire_names() {
        for id in ProviderId::all() {
            assert_eq!(id.to_string().parse::<ProviderId>().expect("parse"), id);
        }
    }
}
