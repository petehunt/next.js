use std::{fmt, sync::Arc};

use futures_util::future::BoxFuture;
use next_rs_core::{Cookie, Error, Request, Response, Result, SameSite};
use next_rs_crypto::SealedBox;
use serde::{Deserialize, Serialize};

use crate::{
    layers::{Clock, SystemClock},
    service::{Middleware, Next},
};

/// Cookie and header names used by the double-submit scheme.
pub const CSRF_COOKIE: &str = "next_rs_csrf";
pub const CSRF_HEADER: &str = "x-next-rs-csrf";

#[derive(Debug, Serialize, Deserialize)]
struct CsrfClaims {
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    session: Option<String>,
    #[serde(rename = "e")]
    expires_at_ms: u64,
}

/// CSRF primitives (spec §93).
///
/// Tokens are authenticated envelopes over an optional session binding plus an
/// expiry, so a token cannot be minted by a client, replayed after expiry, or
/// moved to another session. Validation is double-submit: the same token must
/// arrive in both the cookie and a header, which a cross-site form post cannot
/// arrange.
pub struct CsrfProtection {
    sealed_box: SealedBox,
    ttl_ms: u64,
    clock: Arc<dyn Clock>,
    cookie_name: String,
    header_name: String,
}

impl fmt::Debug for CsrfProtection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CsrfProtection")
            .field("cookie_name", &self.cookie_name)
            .field("header_name", &self.header_name)
            .field("ttl_ms", &self.ttl_ms)
            .finish()
    }
}

impl CsrfProtection {
    /// Creates protection with a process-ephemeral key.
    ///
    /// Use [`CsrfProtection::with_sealed_box`] with a configured key when tokens
    /// must survive a restart or work across several processes.
    pub fn ephemeral() -> Self {
        Self::with_sealed_box(SealedBox::ephemeral("next-rs/csrf"))
    }

    pub fn with_sealed_box(sealed_box: SealedBox) -> Self {
        Self {
            sealed_box,
            ttl_ms: 12 * 60 * 60 * 1000,
            clock: Arc::new(SystemClock),
            cookie_name: CSRF_COOKIE.to_owned(),
            header_name: CSRF_HEADER.to_owned(),
        }
    }

    pub fn with_ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms = ttl_ms.max(1);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_names(mut self, cookie: impl Into<String>, header: impl Into<String>) -> Self {
        self.cookie_name = cookie.into();
        self.header_name = header.into();
        self
    }

    /// Issues a token, optionally bound to a session.
    pub fn issue(&self, session: Option<&str>) -> Result<String> {
        let claims = CsrfClaims {
            session: session.map(str::to_owned),
            expires_at_ms: self.clock.now_ms().saturating_add(self.ttl_ms),
        };
        self.sealed_box.seal(&serde_json::to_vec(&claims)?)
    }

    /// Issues a token and the cookie that carries it.
    ///
    /// The cookie is deliberately not `HttpOnly`: the browser has to read it to
    /// echo it back in the header.
    pub fn issue_cookie(&self, session: Option<&str>) -> Result<(String, Cookie)> {
        let token = self.issue(session)?;
        let cookie = Cookie::new(self.cookie_name.clone(), token.clone())
            .path("/")
            .secure(true)
            .same_site(SameSite::Lax);
        Ok((token, cookie))
    }

    /// Verifies a token against the current session and clock.
    pub fn verify(&self, token: &str, session: Option<&str>) -> Result<()> {
        let plaintext = self
            .sealed_box
            .open(token)
            .map_err(|_| Error::forbidden("invalid CSRF token"))?;
        let claims: CsrfClaims = serde_json::from_slice(&plaintext)
            .map_err(|_| Error::forbidden("invalid CSRF token"))?;

        if self.clock.now_ms() >= claims.expires_at_ms {
            return Err(Error::forbidden("expired CSRF token"));
        }
        match (&claims.session, session) {
            (None, _) => Ok(()),
            (Some(expected), Some(actual)) if expected == actual => Ok(()),
            _ => Err(Error::forbidden("CSRF token does not match this session")),
        }
    }

    /// Validates the double-submit pair on a request.
    pub fn verify_request(&self, request: &Request) -> Result<()> {
        let header = request
            .headers()
            .get(&self.header_name)
            .ok_or_else(|| Error::forbidden("missing CSRF header"))?;
        let cookie = request
            .cookies()
            .get(&self.cookie_name)
            .ok_or_else(|| Error::forbidden("missing CSRF cookie"))?;
        if header != cookie {
            return Err(Error::forbidden("CSRF header does not match the cookie"));
        }
        let session = request
            .session()
            .and_then(next_rs_core::Session::id)
            .map(str::to_owned);
        self.verify(header, session.as_deref())
    }
}

/// Middleware enforcing CSRF protection on unsafe methods.
#[derive(Debug)]
pub struct Csrf {
    protection: Arc<CsrfProtection>,
}

impl Csrf {
    pub fn new(protection: Arc<CsrfProtection>) -> Self {
        Self { protection }
    }

    pub fn protection(&self) -> &Arc<CsrfProtection> {
        &self.protection
    }
}

impl Middleware for Csrf {
    fn handle<'a>(&'a self, request: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response>> {
        Box::pin(async move {
            // Safe methods are not state-changing, so they need no token.
            if request.method().is_safe() {
                return next.run(request).await;
            }
            self.protection.verify_request(&request)?;
            next.run(request).await
        })
    }
}

#[cfg(test)]
mod tests {
    use next_rs_core::{Method, Session, StatusCode};

    use super::*;
    use crate::{layers::ManualClock, service::Stack};

    fn csrf_protection(clock: Arc<ManualClock>) -> CsrfProtection {
        CsrfProtection::ephemeral()
            .with_ttl_ms(1_000)
            .with_clock(clock as Arc<dyn Clock>)
    }

    async fn ok(_request: Request) -> Result<Response> {
        Ok(Response::ok())
    }

    #[test]
    fn round_trips_a_token() {
        let protection = csrf_protection(Arc::new(ManualClock::new(0)));
        let token = protection.issue(None).unwrap();
        assert!(protection.verify(&token, None).is_ok());
    }

    #[test]
    fn tokens_are_cookie_and_header_safe() {
        let token = csrf_protection(Arc::new(ManualClock::new(0)))
            .issue(Some("sess-1"))
            .unwrap();
        assert!(
            token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        );
    }

    #[test]
    fn rejects_a_forged_token() {
        let protection = csrf_protection(Arc::new(ManualClock::new(0)));
        assert!(protection.verify("not-a-token", None).is_err());
        assert!(protection.verify("", None).is_err());

        // A token from a different key never validates.
        let other_protection = csrf_protection(Arc::new(ManualClock::new(0)));
        let token = other_protection.issue(None).unwrap();
        assert!(protection.verify(&token, None).is_err());
    }

    #[test]
    fn rejects_an_expired_token() {
        let clock = Arc::new(ManualClock::new(0));
        let protection = csrf_protection(Arc::clone(&clock));
        let token = protection.issue(None).unwrap();

        clock.advance(999);
        assert!(protection.verify(&token, None).is_ok());
        clock.advance(1);
        let error = protection.verify(&token, None).unwrap_err();
        assert!(error.message().contains("expired"));
    }

    #[test]
    fn enforces_the_session_binding() {
        let protection = csrf_protection(Arc::new(ManualClock::new(0)));
        let token = protection.issue(Some("sess-1")).unwrap();
        assert!(protection.verify(&token, Some("sess-1")).is_ok());
        assert!(protection.verify(&token, Some("sess-2")).is_err());
        assert!(protection.verify(&token, None).is_err());
    }

    #[test]
    fn the_cookie_is_readable_by_script_but_not_cross_site() {
        let (token, cookie) = csrf_protection(Arc::new(ManualClock::new(0)))
            .issue_cookie(None)
            .unwrap();
        let rendered = cookie.to_set_cookie_value();
        assert!(rendered.starts_with(&format!("{CSRF_COOKIE}={token}")));
        assert!(rendered.contains("SameSite=Lax"));
        assert!(rendered.contains("Secure"));
        assert!(!rendered.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn safe_methods_need_no_token() {
        let stack = Stack::new().layer(Csrf::new(Arc::new(csrf_protection(Arc::new(
            ManualClock::new(0),
        )))));
        assert!(
            stack
                .handle(Request::new(Method::Get, "/").unwrap(), &ok)
                .await
                .is_ok()
        );
        assert!(
            stack
                .handle(Request::new(Method::Head, "/").unwrap(), &ok)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn unsafe_methods_require_a_matching_pair() {
        let protection = Arc::new(csrf_protection(Arc::new(ManualClock::new(0))));
        let token = protection.issue(None).unwrap();
        let stack = Stack::new().layer(Csrf::new(Arc::clone(&protection)));

        // Nothing at all.
        let error = stack
            .handle(Request::new(Method::Post, "/").unwrap(), &ok)
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);

        // Header only.
        let request = Request::builder()
            .method("POST")
            .header(CSRF_HEADER, &token)
            .build()
            .unwrap();
        assert!(stack.handle(request, &ok).await.is_err());

        // Mismatched pair.
        let other = protection.issue(None).unwrap();
        let request = Request::builder()
            .method("POST")
            .header(CSRF_HEADER, &token)
            .header("cookie", format!("{CSRF_COOKIE}={other}"))
            .build()
            .unwrap();
        assert!(stack.handle(request, &ok).await.is_err());

        // Matching pair.
        let request = Request::builder()
            .method("POST")
            .header(CSRF_HEADER, &token)
            .header("cookie", format!("{CSRF_COOKIE}={token}"))
            .build()
            .unwrap();
        assert!(stack.handle(request, &ok).await.is_ok());
    }

    #[tokio::test]
    async fn a_token_from_another_session_is_rejected_on_the_request_path() {
        let protection = Arc::new(csrf_protection(Arc::new(ManualClock::new(0))));
        let token = protection.issue(Some("sess-1")).unwrap();
        let stack = Stack::new().layer(Csrf::new(Arc::clone(&protection)));

        let request = Request::builder()
            .method("POST")
            .header(CSRF_HEADER, &token)
            .header("cookie", format!("{CSRF_COOKIE}={token}"))
            .extension(Session::authenticated("sess-2", 7))
            .build()
            .unwrap();
        assert!(stack.handle(request, &ok).await.is_err());

        let request = Request::builder()
            .method("POST")
            .header(CSRF_HEADER, &token)
            .header("cookie", format!("{CSRF_COOKIE}={token}"))
            .extension(Session::authenticated("sess-1", 7))
            .build()
            .unwrap();
        assert!(stack.handle(request, &ok).await.is_ok());
    }

    #[test]
    fn custom_names_are_honoured() {
        let protection = CsrfProtection::ephemeral().with_names("csrf", "x-csrf");
        let (_, cookie) = protection.issue_cookie(None).unwrap();
        assert!(cookie.to_set_cookie_value().starts_with("csrf="));
        assert!(format!("{protection:?}").contains("x-csrf"));
    }
}
