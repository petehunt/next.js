use std::fmt;

/// An HTTP status code.
///
/// A small owned type rather than a re-export so that `next-rs-core` stays
/// independent of any particular HTTP crate version; adapters convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StatusCode(u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const ACCEPTED: Self = Self(202);
    pub const NO_CONTENT: Self = Self(204);
    pub const MOVED_PERMANENTLY: Self = Self(301);
    pub const FOUND: Self = Self(302);
    pub const SEE_OTHER: Self = Self(303);
    pub const NOT_MODIFIED: Self = Self(304);
    pub const TEMPORARY_REDIRECT: Self = Self(307);
    pub const PERMANENT_REDIRECT: Self = Self(308);
    pub const BAD_REQUEST: Self = Self(400);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const FORBIDDEN: Self = Self(403);
    pub const NOT_FOUND: Self = Self(404);
    pub const METHOD_NOT_ALLOWED: Self = Self(405);
    pub const CONFLICT: Self = Self(409);
    pub const GONE: Self = Self(410);
    pub const PAYLOAD_TOO_LARGE: Self = Self(413);
    pub const UNPROCESSABLE_ENTITY: Self = Self(422);
    pub const TOO_MANY_REQUESTS: Self = Self(429);
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);
    pub const NOT_IMPLEMENTED: Self = Self(501);
    pub const BAD_GATEWAY: Self = Self(502);
    pub const SERVICE_UNAVAILABLE: Self = Self(503);
    pub const GATEWAY_TIMEOUT: Self = Self(504);

    /// Creates a status code, rejecting values outside the 100..=599 range.
    pub const fn from_u16(code: u16) -> Option<Self> {
        if code >= 100 && code <= 599 {
            Some(Self(code))
        } else {
            None
        }
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }

    pub const fn is_informational(self) -> bool {
        self.0 >= 100 && self.0 < 200
    }

    pub const fn is_success(self) -> bool {
        self.0 >= 200 && self.0 < 300
    }

    pub const fn is_redirection(self) -> bool {
        self.0 >= 300 && self.0 < 400
    }

    pub const fn is_client_error(self) -> bool {
        self.0 >= 400 && self.0 < 500
    }

    pub const fn is_server_error(self) -> bool {
        self.0 >= 500 && self.0 < 600
    }

    /// True when the status forbids a response body (spec-adjacent HTTP rule
    /// that adapters and the HTML transformer both rely on).
    pub const fn forbids_body(self) -> bool {
        self.0 == 204 || self.0 == 304 || self.is_informational()
    }

    pub const fn canonical_reason(self) -> &'static str {
        match self.0 {
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            204 => "No Content",
            301 => "Moved Permanently",
            302 => "Found",
            303 => "See Other",
            304 => "Not Modified",
            307 => "Temporary Redirect",
            308 => "Permanent Redirect",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            410 => "Gone",
            413 => "Payload Too Large",
            422 => "Unprocessable Entity",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            _ => "",
        }
    }
}

impl Default for StatusCode {
    fn default() -> Self {
        Self::OK
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = self.canonical_reason();
        if reason.is_empty() {
            write!(f, "{}", self.0)
        } else {
            write!(f, "{} {}", self.0, reason)
        }
    }
}

impl TryFrom<u16> for StatusCode {
    type Error = InvalidStatusCode;

    fn try_from(value: u16) -> Result<Self, InvalidStatusCode> {
        Self::from_u16(value).ok_or(InvalidStatusCode(value))
    }
}

/// Error returned when a numeric value is not a valid HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidStatusCode(pub u16);

impl fmt::Display for InvalidStatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid HTTP status code: {}", self.0)
    }
}

impl std::error::Error for InvalidStatusCode {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range_codes() {
        assert!(StatusCode::from_u16(99).is_none());
        assert!(StatusCode::from_u16(600).is_none());
        assert_eq!(StatusCode::from_u16(200), Some(StatusCode::OK));
    }

    #[test]
    fn classifies_ranges() {
        assert!(StatusCode::OK.is_success());
        assert!(StatusCode::FOUND.is_redirection());
        assert!(StatusCode::NOT_FOUND.is_client_error());
        assert!(StatusCode::INTERNAL_SERVER_ERROR.is_server_error());
    }

    #[test]
    fn body_forbidding_statuses() {
        assert!(StatusCode::NO_CONTENT.forbids_body());
        assert!(StatusCode::NOT_MODIFIED.forbids_body());
        assert!(!StatusCode::OK.forbids_body());
    }

    #[test]
    fn displays_reason() {
        assert_eq!(StatusCode::NOT_FOUND.to_string(), "404 Not Found");
        assert_eq!(StatusCode::from_u16(599).unwrap().to_string(), "599");
    }
}
