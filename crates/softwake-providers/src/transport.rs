//! HTTP seam for OAuth and Test probes.
//!
//! Default tests use [`MockTransport`]. Live HTTPS is the `live-http` feature.

use std::collections::HashMap;

use thiserror::Error;

/// One HTTP response the providers code can inspect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Raw body text.
    pub body: String,
}

/// Failure from a [`Transport`] call.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The mock has no route for this request.
    #[error("no mock route for {method} {url}")]
    NoRoute {
        /// HTTP method.
        method: String,
        /// Request URL.
        url: String,
    },
    /// The underlying client could not complete the request.
    #[error("transport failed: {message}")]
    Failed {
        /// Safe display text. Must not include secrets.
        message: String,
    },
}

/// Minimal HTTP client used by OAuth and Test.
pub trait Transport {
    /// POST `application/x-www-form-urlencoded`.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the request cannot complete.
    fn post_form(&self, url: &str, body: &str) -> Result<HttpResponse, TransportError>;

    /// GET with an optional bearer token.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the request cannot complete.
    fn get_bearer(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError>;

    /// POST JSON with a bearer token.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the request cannot complete.
    fn post_json_bearer(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> Result<HttpResponse, TransportError>;
}

/// In-memory routes for unit tests. Does not open a socket.
#[derive(Debug, Default, Clone)]
pub struct MockTransport {
    forms: HashMap<String, HttpResponse>,
    gets: HashMap<String, HttpResponse>,
    posts: HashMap<String, HttpResponse>,
}

impl MockTransport {
    /// Empty mock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a form POST response for `url`.
    #[must_use]
    pub fn with_form(mut self, url: impl Into<String>, response: HttpResponse) -> Self {
        self.forms.insert(url.into(), response);
        self
    }

    /// Register a bearer GET response for `url`.
    #[must_use]
    pub fn with_get(mut self, url: impl Into<String>, response: HttpResponse) -> Self {
        self.gets.insert(url.into(), response);
        self
    }

    /// Register a bearer JSON POST response for `url`.
    #[must_use]
    pub fn with_post_json(mut self, url: impl Into<String>, response: HttpResponse) -> Self {
        self.posts.insert(url.into(), response);
        self
    }
}

impl Transport for MockTransport {
    fn post_form(&self, url: &str, _body: &str) -> Result<HttpResponse, TransportError> {
        self.forms
            .get(url)
            .cloned()
            .ok_or_else(|| TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
    }

    fn get_bearer(&self, url: &str, _bearer: &str) -> Result<HttpResponse, TransportError> {
        self.gets
            .get(url)
            .cloned()
            .ok_or_else(|| TransportError::NoRoute {
                method: "GET".to_owned(),
                url: url.to_owned(),
            })
    }

    fn post_json_bearer(
        &self,
        url: &str,
        _bearer: &str,
        _body: &str,
    ) -> Result<HttpResponse, TransportError> {
        self.posts
            .get(url)
            .cloned()
            .ok_or_else(|| TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
    }
}
