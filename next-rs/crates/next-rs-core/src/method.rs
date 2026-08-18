use std::fmt;

/// An HTTP method.
///
/// `route.rs` exports one function per method (`GET`, `POST`, ...) per spec §17,
/// so the router needs a cheap, case-normalising method type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Method {
    #[default]
    Get,
    Head,
    Post,
    Put,
    Delete,
    Connect,
    Options,
    Trace,
    Patch,
    /// A method outside the registered set, stored uppercased.
    Other(String),
}

impl Method {
    /// Parses a method name. Standard methods are matched case-insensitively so
    /// that `route.rs` exports (`GET`) and wire values (`get`) agree.
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "DELETE" => Self::Delete,
            "CONNECT" => Self::Connect,
            "OPTIONS" => Self::Options,
            "TRACE" => Self::Trace,
            "PATCH" => Self::Patch,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Connect => "CONNECT",
            Self::Options => "OPTIONS",
            Self::Trace => "TRACE",
            Self::Patch => "PATCH",
            Self::Other(value) => value,
        }
    }

    /// True for methods that must not carry a response body.
    pub fn forbids_response_body(&self) -> bool {
        matches!(self, Self::Head)
    }

    /// True for methods defined as safe by RFC 9110.
    pub fn is_safe(&self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options | Self::Trace)
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Method {
    fn from(value: &str) -> Self {
        Self::parse(value)
    }
}

impl From<String> for Method {
    fn from(value: String) -> Self {
        Self::parse(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_case_insensitively() {
        assert_eq!(Method::parse("get"), Method::Get);
        assert_eq!(Method::parse("GET"), Method::Get);
        assert_eq!(Method::parse("PaTcH"), Method::Patch);
    }

    #[test]
    fn preserves_unknown_methods_uppercased() {
        assert_eq!(Method::parse("purge"), Method::Other("PURGE".to_owned()));
        assert_eq!(Method::parse("purge").as_str(), "PURGE");
    }

    #[test]
    fn head_forbids_response_body() {
        assert!(Method::Head.forbids_response_body());
        assert!(!Method::Get.forbids_response_body());
    }
}
