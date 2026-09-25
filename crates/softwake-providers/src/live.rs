//! Live HTTPS transport behind the `live-http` feature.
//!
//! Default CI does not compile this module's dependency path unless the
//! feature is enabled. Unit tests stay on [`crate::transport::MockTransport`].

use std::time::Duration;

use ureq::Agent;

use crate::transport::{HttpResponse, Transport, TransportError};

/// `ureq`-backed transport for manual demos and ignored live tests.
#[derive(Debug, Clone)]
pub struct LiveTransport {
    agent: Agent,
}

impl Default for LiveTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveTransport {
    /// Build a transport with the default agent.
    ///
    /// The default agent sets a connect timeout and leaves the read timeout
    /// and the overall timeout unset. Settings Test uses this. Chat uses
    /// [`Self::bounded`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            agent: Agent::new(),
        }
    }

    /// Connect, read, and overall timeout all set to `timeout`.
    ///
    /// A silent 200 cannot block past `timeout`.
    #[must_use]
    pub fn bounded(timeout: Duration) -> Self {
        let agent = ureq::builder()
            .timeout_connect(timeout)
            .timeout_read(timeout)
            .timeout(timeout)
            .build();
        Self { agent }
    }
}

impl Transport for LiveTransport {
    fn post_form(&self, url: &str, body: &str) -> Result<HttpResponse, TransportError> {
        let response = self
            .agent
            .post(url)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(body)
            .map_err(|error| TransportError::Failed {
                message: safe_ureq_message(&error),
            })?;
        read_response(response)
    }

    fn get_bearer(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
        let response = self
            .agent
            .get(url)
            .set("Authorization", &format!("Bearer {bearer}"))
            .call()
            .map_err(|error| TransportError::Failed {
                message: safe_ureq_message(&error),
            })?;
        read_response(response)
    }

    fn post_json_bearer(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> Result<HttpResponse, TransportError> {
        let response = self
            .agent
            .post(url)
            .set("Authorization", &format!("Bearer {bearer}"))
            .set("Content-Type", "application/json")
            .send_string(body)
            .map_err(|error| TransportError::Failed {
                message: safe_ureq_message(&error),
            })?;
        read_response(response)
    }
}

fn read_response(response: ureq::Response) -> Result<HttpResponse, TransportError> {
    let status = response.status();
    let body = response
        .into_string()
        .map_err(|error| TransportError::Failed {
            message: format!("failed to read response body: {error}"),
        })?;
    Ok(HttpResponse { status, body })
}

fn safe_ureq_message(error: &ureq::Error) -> String {
    // Do not include request bodies; they may hold tokens.
    match error {
        ureq::Error::Status(code, _) => format!("HTTP status {code}"),
        ureq::Error::Transport(_) => "network transport failed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    //! Live tests are ignored so default CI stays offline.

    #[test]
    #[ignore = "live network; enable with --features live-http -- --ignored"]
    fn live_feature_compiles() {
        let _ = super::LiveTransport::new();
    }
}
