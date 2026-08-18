use std::{any::Any, fmt, future::Future, sync::Arc};

use futures_util::future::BoxFuture;
use next_rs_core::{CookieJar, Error, Extensions, HeaderMap, Result, Session};

/// The first parameter of every `#[react_component]` loader (spec §28).
///
/// The runtime reconstructs a fresh context on every invocation — initial render
/// *and* every SWR refresh — which is why authorization can be re-run each time
/// (spec §61). A context is never serialised into slot state (spec §58).
#[derive(Clone)]
pub struct RenderContext {
    headers: Arc<HeaderMap>,
    cookies: Arc<CookieJar>,
    session: Option<Arc<Session>>,
    request_id: Arc<str>,
    extensions: Arc<Extensions>,
    state: Option<Arc<dyn Any + Send + Sync>>,
    /// Authorization surface used as `ctx.auth.require_user(...)` in the spec.
    pub auth: Authorization,
}

impl RenderContext {
    pub fn builder() -> RenderContextBuilder {
        RenderContextBuilder::default()
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn cookies(&self) -> &CookieJar {
        &self.cookies
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_deref()
    }

    /// The session identifier used to bind slot tokens (spec §57).
    pub fn session_binding(&self) -> Option<&str> {
        self.session.as_deref().and_then(Session::id)
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Rust-only request state carried from middleware (spec §16, §69).
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Shared application state — database pools, permission engines, native
    /// handles (spec §69). These never become React props.
    pub fn state<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.state.as_ref()?.downcast_ref::<T>()
    }

    /// Same as [`RenderContext::state`] but fails loudly, which is usually what
    /// a loader wants: a missing pool is a wiring bug, not a request error.
    pub fn require_state<T: Any + Send + Sync>(&self) -> Result<&T> {
        self.state::<T>().ok_or_else(|| {
            Error::internal(format!(
                "application state `{}` is not installed on the render context",
                std::any::type_name::<T>()
            ))
        })
    }

    /// Runs `future` with this context installed as the ambient render context,
    /// so `HTML::render(...)` and slot scheduling can find it.
    pub async fn scope<F: Future>(self, future: F) -> F::Output {
        RENDER_CONTEXT.scope(self, future).await
    }
}

impl fmt::Debug for RenderContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderContext")
            .field("request_id", &self.request_id)
            .field("header_names", &self.headers.len())
            .field("authenticated", &self.session.is_some())
            .field("has_state", &self.state.is_some())
            .finish()
    }
}

/// Builder for [`RenderContext`].
#[derive(Default)]
pub struct RenderContextBuilder {
    headers: HeaderMap,
    cookies: Option<CookieJar>,
    session: Option<Session>,
    request_id: Option<String>,
    extensions: Option<Arc<Extensions>>,
    state: Option<Arc<dyn Any + Send + Sync>>,
    policy: Option<Arc<dyn AuthPolicy>>,
}

impl fmt::Debug for RenderContextBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderContextBuilder")
            .finish_non_exhaustive()
    }
}

impl RenderContextBuilder {
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    pub fn cookies(mut self, cookies: CookieJar) -> Self {
        self.cookies = Some(cookies);
        self
    }

    pub fn session(mut self, session: Session) -> Self {
        self.session = Some(session);
        self
    }

    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn extensions(mut self, extensions: Arc<Extensions>) -> Self {
        self.extensions = Some(extensions);
        self
    }

    pub fn state<T: Any + Send + Sync>(mut self, state: Arc<T>) -> Self {
        self.state = Some(state);
        self
    }

    /// Installs the application's authorization policy. Without one, every
    /// `ctx.auth` check is denied.
    pub fn auth_policy(mut self, policy: Arc<dyn AuthPolicy>) -> Self {
        self.policy = Some(policy);
        self
    }

    pub fn build(self) -> RenderContext {
        let cookies = self
            .cookies
            .unwrap_or_else(|| CookieJar::from_headers(&self.headers));
        RenderContext {
            headers: Arc::new(self.headers),
            cookies: Arc::new(cookies),
            session: self.session.map(Arc::new),
            request_id: Arc::from(self.request_id.unwrap_or_default().as_str()),
            extensions: self.extensions.unwrap_or_default(),
            state: self.state,
            auth: Authorization {
                policy: self.policy.unwrap_or_else(|| Arc::new(DenyAll)),
                session: None,
            },
        }
        .with_auth_session()
    }
}

impl RenderContext {
    /// Threads the resolved session into the authorization surface so policies
    /// see who is asking without a second lookup.
    fn with_auth_session(mut self) -> Self {
        self.auth.session = self.session.clone();
        self
    }
}

tokio::task_local! {
    static RENDER_CONTEXT: RenderContext;
}

/// The ambient render context, when one is installed.
///
/// `HTML::render(...)` takes no context parameter (spec §22), so the runtime
/// installs one around the route handler.
pub fn current_render_context() -> Option<RenderContext> {
    RENDER_CONTEXT.try_with(Clone::clone).ok()
}

/// Extension trait for installing a render context around a future.
pub trait WithRenderContext: Future + Sized {
    /// Runs `self` with `context` installed.
    fn with_render_context(
        self,
        context: RenderContext,
    ) -> tokio::task::futures::TaskLocalFuture<RenderContext, Self> {
        RENDER_CONTEXT.scope(context, self)
    }
}

impl<F: Future + Sized> WithRenderContext for F {}

/// Who or what an authorization check is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    /// Application-defined subject kind, e.g. `user` or `org`.
    pub kind: String,
    pub id: SubjectId,
}

impl Subject {
    pub fn new(kind: impl Into<String>, id: SubjectId) -> Self {
        Self {
            kind: kind.into(),
            id,
        }
    }

    pub fn user(id: u64) -> Self {
        Self::new("user", SubjectId::Numeric(id))
    }

    pub fn org(id: u64) -> Self {
        Self::new("org", SubjectId::Numeric(id))
    }
}

/// A subject identifier. Numeric and string IDs are both common.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectId {
    Numeric(u64),
    Text(String),
}

impl fmt::Display for SubjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Numeric(value) => write!(f, "{value}"),
            Self::Text(value) => f.write_str(value),
        }
    }
}

impl From<u64> for SubjectId {
    fn from(value: u64) -> Self {
        Self::Numeric(value)
    }
}

impl From<String> for SubjectId {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for SubjectId {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

/// A single authorization question.
#[derive(Debug, Clone)]
pub struct AuthRequest {
    /// Application-defined permission name, e.g. `read`.
    pub permission: String,
    pub subject: Subject,
    /// The session the check runs under, if any.
    pub session: Option<Session>,
}

/// Reason an authorization check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// No credentials at all — a 401.
    Unauthenticated,
    /// Credentials present but insufficient — a 403.
    Denied,
}

impl AuthError {
    pub fn into_error(self, subject: &Subject) -> Error {
        match self {
            Self::Unauthenticated => Error::unauthorized(format!(
                "authentication required to access {} {}",
                subject.kind, subject.id
            )),
            Self::Denied => {
                Error::forbidden(format!("access denied to {} {}", subject.kind, subject.id))
            }
        }
    }
}

/// The application's authorization policy.
///
/// Authorization is *always* re-run: encrypted slot arguments are not a grant
/// (spec §61). The framework therefore never caches a policy decision.
pub trait AuthPolicy: Send + Sync + 'static {
    fn authorize<'a>(
        &'a self,
        request: &'a AuthRequest,
    ) -> BoxFuture<'a, std::result::Result<(), AuthError>>;
}

/// The default policy: deny everything.
///
/// A missing policy must not mean "allow" — a loader that forgets to install one
/// should fail closed.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAll;

impl AuthPolicy for DenyAll {
    fn authorize<'a>(
        &'a self,
        _request: &'a AuthRequest,
    ) -> BoxFuture<'a, std::result::Result<(), AuthError>> {
        Box::pin(async { Err(AuthError::Denied) })
    }
}

/// `ctx.auth` — the authorization surface used by loaders (spec §26, §61).
#[derive(Clone)]
pub struct Authorization {
    policy: Arc<dyn AuthPolicy>,
    session: Option<Arc<Session>>,
}

impl Authorization {
    pub fn new(policy: Arc<dyn AuthPolicy>) -> Self {
        Self {
            policy,
            session: None,
        }
    }

    /// Runs an arbitrary check against the policy.
    pub async fn require(&self, permission: &str, subject: Subject) -> Result<()> {
        let request = AuthRequest {
            permission: permission.to_owned(),
            subject,
            session: self.session.as_deref().cloned(),
        };
        self.policy
            .authorize(&request)
            .await
            .map_err(|error| error.into_error(&request.subject))
    }

    /// `ctx.auth.require_user(user_id)` from spec §95.
    pub async fn require_user(&self, user_id: u64) -> Result<()> {
        self.require("read", Subject::user(user_id)).await
    }

    /// `ctx.auth.require_access_to_org(org_id)` from spec §26.
    pub async fn require_access_to_org(&self, org_id: u64) -> Result<()> {
        self.require("read", Subject::org(org_id)).await
    }
}

impl fmt::Debug for Authorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authorization")
            .field("has_session", &self.session.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct AllowOrgs(Vec<u64>);

    impl AuthPolicy for AllowOrgs {
        fn authorize<'a>(
            &'a self,
            request: &'a AuthRequest,
        ) -> BoxFuture<'a, std::result::Result<(), AuthError>> {
            Box::pin(async move {
                if request.session.is_none() {
                    return Err(AuthError::Unauthenticated);
                }
                match (&request.subject.kind[..], &request.subject.id) {
                    ("org", SubjectId::Numeric(id)) if self.0.contains(id) => Ok(()),
                    _ => Err(AuthError::Denied),
                }
            })
        }
    }

    #[derive(Debug, PartialEq)]
    struct Pool(u32);

    fn context() -> RenderContext {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "sid=abc");
        headers.insert("x-request-id", "req-1");
        RenderContext::builder()
            .headers(headers)
            .session(Session::authenticated("sess-1", 7))
            .request_id("req-1")
            .state(Arc::new(Pool(5)))
            .auth_policy(Arc::new(AllowOrgs(vec![42])))
            .build()
    }

    #[test]
    fn exposes_request_metadata() {
        let context = context();
        assert_eq!(context.request_id(), "req-1");
        assert_eq!(context.headers().get("x-request-id"), Some("req-1"));
        // Cookies are derived from headers when not supplied explicitly.
        assert_eq!(context.cookies().get("sid"), Some("abc"));
        assert_eq!(context.session().unwrap().user_id().unwrap(), 7);
        assert_eq!(context.session_binding(), Some("sess-1"));
    }

    #[test]
    fn exposes_application_state() {
        let context = context();
        assert_eq!(context.state::<Pool>(), Some(&Pool(5)));
        assert_eq!(context.require_state::<Pool>().unwrap(), &Pool(5));
        assert!(context.state::<String>().is_none());
        assert!(context.require_state::<String>().is_err());
    }

    #[tokio::test]
    async fn authorization_consults_the_policy() {
        let context = context();
        assert!(context.auth.require_access_to_org(42).await.is_ok());

        let error = context.auth.require_access_to_org(9).await.unwrap_err();
        assert_eq!(error.status().as_u16(), 403);
    }

    #[tokio::test]
    async fn anonymous_requests_are_unauthenticated_not_forbidden() {
        let context = RenderContext::builder()
            .auth_policy(Arc::new(AllowOrgs(vec![42])))
            .build();
        let error = context.auth.require_access_to_org(42).await.unwrap_err();
        assert_eq!(error.status().as_u16(), 401);
    }

    #[tokio::test]
    async fn the_default_policy_denies() {
        let context = RenderContext::builder().build();
        assert!(context.auth.require_user(1).await.is_err());
        assert!(context.auth.require_access_to_org(1).await.is_err());
        assert_eq!(context.session_binding(), None);
    }

    #[tokio::test]
    async fn render_context_is_ambient_within_a_scope() {
        assert!(current_render_context().is_none());

        let context = context();
        let observed = context
            .clone()
            .scope(async { current_render_context().map(|ctx| ctx.request_id().to_owned()) })
            .await;
        assert_eq!(observed.as_deref(), Some("req-1"));

        // The scope does not leak out.
        assert!(current_render_context().is_none());
    }

    #[tokio::test]
    async fn with_render_context_extension_trait_works() {
        let context = context();
        let observed = async { current_render_context().is_some() }
            .with_render_context(context)
            .await;
        assert!(observed);
    }

    #[test]
    fn subject_ids_display_uniformly() {
        assert_eq!(SubjectId::Numeric(4).to_string(), "4");
        assert_eq!(SubjectId::from("abc").to_string(), "abc");
        assert_eq!(Subject::org(1).kind, "org");
    }

    #[test]
    fn debug_does_not_dump_request_content() {
        let rendered = format!("{:?}", context());
        assert!(rendered.contains("req-1"));
        assert!(!rendered.contains("sid=abc"));
    }
}
