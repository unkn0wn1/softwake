//! Live HTTPS transport behind the `live-http` feature.
//!
//! Default CI does not compile this module's dependency path unless the
//! feature is enabled. Unit tests stay on [`crate::transport::MockTransport`].

use std::io::Read;
use std::time::Duration;

use ureq::Agent;

use crate::transport::{HttpBytes, HttpResponse, MultipartField, Transport, TransportError};

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
        let response = take_response(
            self.agent
                .post(url)
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string(body),
        )?;
        read_response(response)
    }

    fn get_bearer(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
        let response = take_response(
            self.agent
                .get(url)
                .set("Authorization", &format!("Bearer {bearer}"))
                .call(),
        )?;
        read_response(response)
    }

    fn post_json_bearer(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> Result<HttpResponse, TransportError> {
        let response = take_response(
            self.agent
                .post(url)
                .set("Authorization", &format!("Bearer {bearer}"))
                .set("Content-Type", "application/json")
                .send_string(body),
        )?;
        read_response(response)
    }

    fn post_multipart_bearer(
        &self,
        url: &str,
        bearer: &str,
        fields: &[MultipartField],
        file_name: &str,
        file_bytes: &[u8],
        file_content_type: &str,
    ) -> Result<HttpResponse, TransportError> {
        // ureq 2 has no multipart builder. Text fields are written first and
        // the file part last, matching the xAI STT order.
        let response = send_multipart(
            self,
            url,
            bearer,
            fields,
            file_name,
            file_bytes,
            file_content_type,
        )?;
        read_response(response)
    }

    fn post_json_bearer_bytes(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> Result<HttpBytes, TransportError> {
        let response = take_response(
            self.agent
                .post(url)
                .set("Authorization", &format!("Bearer {bearer}"))
                .set("Content-Type", "application/json")
                .send_string(body),
        )?;
        read_bytes(response)
    }
}

fn send_multipart(
    transport: &LiveTransport,
    url: &str,
    bearer: &str,
    fields: &[MultipartField],
    file_name: &str,
    file_bytes: &[u8],
    file_content_type: &str,
) -> Result<ureq::Response, TransportError> {
    let boundary = format!("softwake{}", std::process::id());
    let mut body = Vec::new();
    for field in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                field.name, field.value
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: {file_content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    take_response(
        transport
            .agent
            .post(url)
            .set("Authorization", &format!("Bearer {bearer}"))
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body),
    )
}

fn read_bytes(response: ureq::Response) -> Result<HttpBytes, TransportError> {
    let status = response.status();
    let mut body = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|error| TransportError::Failed {
            message: format!("failed to read response body: {error}"),
        })?;
    Ok(HttpBytes { status, body })
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

/// Prefer returning the HTTP response (including 4xx/5xx) over collapsing every
/// non-2xx into [`TransportError`]. Voice/TTS maps status via parse helpers;
/// treating 401/403 as "unreachable" hid expired OAuth for state announcements.
fn take_response(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<ureq::Response, TransportError> {
    match result {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(_code, response)) => Ok(response),
        Err(error) => Err(TransportError::Failed {
            message: safe_ureq_message(&error),
        }),
    }
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
