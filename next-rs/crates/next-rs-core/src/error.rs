use std::fmt;

use crate::status::StatusCode;

/// The `next-rs` result alias.
///
/// Spec examples write `Result<Response>` and `Result<DashboardProps>`, so the
/// error parameter defaults to [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Classification of a `next-rs` error, used to derive an HTTP status and a
/// stable machine-readable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Conflict,
    PayloadTooLarge,
    TooManyRequests,
    Timeout,
    Serialization,
    Crypto,
    /// A slot token was issued by a different build (spec §66).
    StaleBuild,
    Internal,
}

impl ErrorKind {
    pub const fn status(self) -> StatusCode {
        match self {
            Self::BadRequest | Self::Serialization => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            // A token that fails authenticated decryption is not a hint to
            // retry with different credentials; it is a rejected payload.
            Self::Forbidden | Self::Crypto => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::Conflict | Self::StaleBuild => StatusCode::CONFLICT,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable code reported to the browser runtime.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BadRequest => "BAD_REQUEST",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::NotFound => "NOT_FOUND",
            Self::MethodNotAllowed => "METHOD_NOT_ALLOWED",
            Self::Conflict => "CONFLICT",
            Self::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            Self::TooManyRequests => "TOO_MANY_REQUESTS",
            Self::Timeout => "TIMEOUT",
            Self::Serialization => "SERIALIZATION",
            Self::Crypto => "CRYPTO",
            Self::StaleBuild => "STALE_BUILD",
            Self::Internal => "INTERNAL",
        }
    }

    /// Whether the message is safe to expose to the browser.
    ///
    /// Internal and crypto failures are deliberately opaque so that loader
    /// internals and token details never leak.
    pub const fn message_is_public(self) -> bool {
        !matches!(self, Self::Internal | Self::Crypto)
    }
}

/// A `next-rs` error.
#[derive(Debug, thiserror::Error)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// Attaches a cause for logging. The cause is never rendered into a
    /// response body.
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::BadRequest, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unauthorized, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Forbidden, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::MethodNotAllowed, message)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::PayloadTooLarge, message)
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::TooManyRequests, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Timeout, message)
    }

    pub fn serialization(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Serialization, message)
    }

    pub fn crypto(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Crypto, message)
    }

    pub fn stale_build(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::StaleBuild, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn status(&self) -> StatusCode {
        self.kind.status()
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// The message safe to send to a client, redacted for internal failures.
    pub fn public_message(&self) -> &str {
        if self.kind.message_is_public() {
            &self.message
        } else {
            match self.kind {
                ErrorKind::Crypto => "invalid token",
                _ => "internal server error",
            }
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.code(), self.message)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        let message = value.to_string();
        Self::new(ErrorKind::Serialization, message).with_source(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        let kind = match value.kind() {
            std::io::ErrorKind::TimedOut => ErrorKind::Timeout,
            std::io::ErrorKind::NotFound => ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorKind::Forbidden,
            _ => ErrorKind::Internal,
        };
        let message = value.to_string();
        Self::new(kind, message).with_source(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_kinds_to_statuses() {
        assert_eq!(Error::bad_request("x").status(), StatusCode::BAD_REQUEST);
        assert_eq!(Error::stale_build("x").status(), StatusCode::CONFLICT);
        assert_eq!(Error::crypto("x").status(), StatusCode::FORBIDDEN);
        assert_eq!(
            Error::internal("x").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn redacts_internal_and_crypto_messages() {
        let error = Error::internal("connection string: postgres://secret");
        assert_eq!(error.public_message(), "internal server error");

        let error = Error::crypto("aead tag mismatch at offset 12");
        assert_eq!(error.public_message(), "invalid token");

        let error = Error::bad_request("missing query parameter `org_id`");
        assert_eq!(error.public_message(), "missing query parameter `org_id`");
    }

    #[test]
    fn converts_from_serde_json() {
        let json_error = serde_json::from_str::<u32>("not json").unwrap_err();
        let error = Error::from(json_error);
        assert_eq!(error.kind(), ErrorKind::Serialization);
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn stale_build_has_stable_code() {
        assert_eq!(Error::stale_build("x").code(), "STALE_BUILD");
    }
}
