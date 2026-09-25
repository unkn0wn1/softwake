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

/// One HTTP response whose body is raw bytes (TTS audio).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBytes {
    /// Status code.
    pub status: u16,
    /// Raw body bytes. Empty when the server sent none.
    pub body: Vec<u8>,
}

/// One multipart text field. The file part is separate so the body stays last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartField {
    /// Form field name.
    pub name: String,
    /// Field value. Not a file.
    pub value: String,
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

    /// POST `multipart/form-data` with a bearer token.
    ///
    /// Text `fields` are written first. `file` is the last part (`name` is the
    /// form field, typically `file`). STT requires that order.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the request cannot complete.
    fn post_multipart_bearer(
        &self,
        url: &str,
        bearer: &str,
        fields: &[MultipartField],
        file_name: &str,
        file_bytes: &[u8],
        file_content_type: &str,
    ) -> Result<HttpResponse, TransportError>;

    /// POST JSON with a bearer token and return the raw body bytes.
    ///
    /// Used for TTS, which answers with audio rather than JSON text.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the request cannot complete.
    fn post_json_bearer_bytes(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> Result<HttpBytes, TransportError>;
}

/// In-memory routes for unit tests. Does not open a socket.
#[derive(Debug, Default)]
pub struct MockTransport {
    forms: HashMap<String, HttpResponse>,
    gets: HashMap<String, HttpResponse>,
    posts: HashMap<String, HttpResponse>,
    multiparts: HashMap<String, HttpResponse>,
    byte_posts: HashMap<String, HttpBytes>,
    /// Last multipart call, for tests that assert request shape.
    last_multipart: std::cell::RefCell<Option<RecordedMultipart>>,
    /// Last binary JSON POST body, for tests that assert TTS JSON.
    last_byte_post: std::cell::RefCell<Option<RecordedBytePost>>,
}

/// One recorded multipart POST. The bearer is stored so tests can assert it
/// was forwarded; production logs must not print it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedMultipart {
    /// Request URL.
    pub url: String,
    /// Bearer token the caller passed.
    pub bearer: String,
    /// Text fields, in order.
    pub fields: Vec<MultipartField>,
    /// File field name.
    pub file_name: String,
    /// File bytes.
    pub file_bytes: Vec<u8>,
    /// File content type.
    pub file_content_type: String,
}

/// One recorded JSON POST that expected bytes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedBytePost {
    /// Request URL.
    pub url: String,
    /// Bearer token the caller passed.
    pub bearer: String,
    /// JSON body.
    pub body: String,
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

    /// Register a multipart POST response for `url`.
    #[must_use]
    pub fn with_multipart(mut self, url: impl Into<String>, response: HttpResponse) -> Self {
        self.multiparts.insert(url.into(), response);
        self
    }

    /// Register a binary JSON POST response for `url`.
    #[must_use]
    pub fn with_post_bytes(mut self, url: impl Into<String>, response: HttpBytes) -> Self {
        self.byte_posts.insert(url.into(), response);
        self
    }

    /// Last multipart request, if any.
    #[must_use]
    pub fn last_multipart(&self) -> Option<RecordedMultipart> {
        self.last_multipart.borrow().clone()
    }

    /// Last binary JSON POST, if any.
    #[must_use]
    pub fn last_byte_post(&self) -> Option<RecordedBytePost> {
        self.last_byte_post.borrow().clone()
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

    fn post_multipart_bearer(
        &self,
        url: &str,
        bearer: &str,
        fields: &[MultipartField],
        file_name: &str,
        file_bytes: &[u8],
        file_content_type: &str,
    ) -> Result<HttpResponse, TransportError> {
        *self.last_multipart.borrow_mut() = Some(RecordedMultipart {
            url: url.to_owned(),
            bearer: bearer.to_owned(),
            fields: fields.to_vec(),
            file_name: file_name.to_owned(),
            file_bytes: file_bytes.to_vec(),
            file_content_type: file_content_type.to_owned(),
        });
        self.multiparts
            .get(url)
            .cloned()
            .ok_or_else(|| TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
    }

    fn post_json_bearer_bytes(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> Result<HttpBytes, TransportError> {
        *self.last_byte_post.borrow_mut() = Some(RecordedBytePost {
            url: url.to_owned(),
            bearer: bearer.to_owned(),
            body: body.to_owned(),
        });
        self.byte_posts
            .get(url)
            .cloned()
            .ok_or_else(|| TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
    }
}
