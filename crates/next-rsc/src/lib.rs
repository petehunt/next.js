//! Experimental authoring model and host ABI for Rust Server Components.
//!
//! The API is intentionally small while `layout.rs` integration is prototyped.
//! It models React children as opaque slots and never exposes JavaScript values.
//!
//! # Component ABI
//!
//! Hosts send a bounded UTF-8 input envelope containing `abiVersion`, params,
//! and slot IDs. Components return the JSON produced by [`encode_render_result`].
//! Both sides must reject an unknown [`ABI_VERSION`] before rendering. Buffers
//! are caller-owned: the allocating side also frees them, and lengths are always
//! validated before a pointer is dereferenced. Additive SDK changes do not bump
//! the ABI; an incompatible envelope or node encoding does.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt::Write,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(feature = "macros")]
pub use next_rsc_macros::{layout, page};

pub const ABI_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FetchCacheMode {
    #[default]
    Uncached,
    Request,
    Warm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchRequest {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub cache: FetchCacheMode,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub overall_timeout: Duration,
    pub max_response_bytes: usize,
}

impl FetchRequest {
    pub fn get(url: impl Into<String>) -> Result<Self, RenderError> {
        let url = url.into();
        if !(url.starts_with("http://") || url.starts_with("https://"))
            || url.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(RenderError::new(
                "fetch URL must be an absolute HTTP(S) URL",
            ));
        }
        Ok(Self {
            url,
            headers: BTreeMap::new(),
            cache: FetchCacheMode::Uncached,
            connect_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(3),
            overall_timeout: Duration::from_secs(5),
            max_response_bytes: 2 * 1024 * 1024,
        })
    }

    pub fn header(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, RenderError> {
        let name = name.into();
        let value = value.into();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.bytes().any(|byte| matches!(byte, b'\r' | b'\n'))
        {
            return Err(RenderError::new("invalid fetch header"));
        }
        self.headers.insert(name, value);
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub cache: FetchCacheMode,
}

pub type FetchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<FetchResponse, RenderError>> + Send + 'a>>;

pub trait FetchHost: Send + Sync {
    fn fetch(&self, request: FetchRequest, cancelled: Arc<AtomicBool>) -> FetchFuture<'_>;
}

#[derive(Clone)]
pub struct RequestContext {
    host: Arc<dyn FetchHost>,
    cancelled: Arc<AtomicBool>,
}

impl RequestContext {
    pub fn new(host: Arc<dyn FetchHost>, cancelled: Arc<AtomicBool>) -> Self {
        Self { host, cancelled }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, RenderError> {
        if self.is_cancelled() {
            return Err(RenderError::new("request cancelled"));
        }
        let response = self
            .host
            .fetch(request.clone(), Arc::clone(&self.cancelled))
            .await?;
        if response.body.len() > request.max_response_bytes {
            return Err(RenderError::new("fetch response exceeded byte limit"));
        }
        Ok(response)
    }
    #[cfg(feature = "json")]
    pub async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        request: FetchRequest,
    ) -> Result<T, RenderError> {
        let response = self.fetch(request).await?;
        if !(200..300).contains(&response.status) {
            return Err(RenderError::new(format!(
                "fetch returned HTTP {}",
                response.status
            )));
        }
        serde_json::from_slice(&response.body)
            .map_err(|error| RenderError::new(format!("fetch JSON decode failed: {error}")))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub struct SlotId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    String(String),
    Strings(Vec<String>),
    Style(BTreeMap<String, String>),
    Node(Box<Node>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub tag: String,
    pub props: BTreeMap<String, PropValue>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClientReference {
    pub module_id: String,
    pub export_name: String,
    pub chunks: Vec<String>,
    pub asynchronous: bool,
    pub props: BTreeMap<String, PropValue>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Null,
    Text(String),
    Element(Element),
    ClientReference(ClientReference),
    Fragment(Vec<Node>),
    ReactFragment {
        key: Option<String>,
        children: Vec<Node>,
    },
    Suspense {
        fallback: Box<Node>,
        content: Box<Node>,
    },
    Slot(SlotId),
}

impl Node {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn fragment(children: impl IntoIterator<Item = Node>) -> Self {
        Self::Fragment(children.into_iter().collect())
    }

    pub fn react_fragment(
        key: Option<impl Into<String>>,
        children: impl IntoIterator<Item = Node>,
    ) -> Self {
        Self::ReactFragment {
            key: key.map(Into::into),
            children: children.into_iter().collect(),
        }
    }

    pub fn slot(id: u32) -> Self {
        Self::Slot(SlotId(id))
    }

    pub fn suspense(fallback: Node, content: Node) -> Self {
        Self::Suspense {
            fallback: Box::new(fallback),
            content: Box::new(content),
        }
    }

    pub fn prop(mut self, name: impl Into<String>, value: impl Into<PropValue>) -> Self {
        match &mut self {
            Self::Element(element) => {
                element.props.insert(name.into(), value.into());
            }
            Self::ClientReference(reference) => {
                reference.props.insert(name.into(), value.into());
            }
            _ => {}
        }
        self
    }

    pub fn replace_slot(self, slot: SlotId, replacement: &Node) -> Self {
        match self {
            Self::Slot(current) if current == slot => replacement.clone(),
            Self::Element(mut element) => {
                element.children = element
                    .children
                    .into_iter()
                    .map(|child| child.replace_slot(slot, replacement))
                    .collect();
                Self::Element(element)
            }
            Self::ClientReference(mut reference) => {
                reference.children = reference
                    .children
                    .into_iter()
                    .map(|child| child.replace_slot(slot, replacement))
                    .collect();
                Self::ClientReference(reference)
            }
            Self::Fragment(children) => Self::Fragment(
                children
                    .into_iter()
                    .map(|child| child.replace_slot(slot, replacement))
                    .collect(),
            ),
            Self::ReactFragment { key, children } => Self::ReactFragment {
                key,
                children: children
                    .into_iter()
                    .map(|child| child.replace_slot(slot, replacement))
                    .collect(),
            },
            Self::Suspense { fallback, content } => Self::Suspense {
                fallback: Box::new(fallback.replace_slot(slot, replacement)),
                content: Box::new(content.replace_slot(slot, replacement)),
            },
            node => node,
        }
    }
}

impl From<&str> for Node {
    fn from(value: &str) -> Self {
        Self::text(value)
    }
}

impl From<String> for Node {
    fn from(value: String) -> Self {
        Self::text(value)
    }
}

impl From<&str> for PropValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for PropValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<bool> for PropValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for PropValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

pub fn element(tag: impl Into<String>, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Element(Element {
        tag: tag.into(),
        props: BTreeMap::new(),
        children: children.into_iter().collect(),
    })
}

pub fn client_reference(
    module_id: impl Into<String>,
    export_name: impl Into<String>,
    chunks: impl IntoIterator<Item = impl Into<String>>,
    children: impl IntoIterator<Item = Node>,
) -> Node {
    Node::ClientReference(ClientReference {
        module_id: module_id.into(),
        export_name: export_name.into(),
        chunks: chunks.into_iter().map(Into::into).collect(),
        asynchronous: false,
        props: BTreeMap::new(),
        children: children.into_iter().collect(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IrLimits {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_string_bytes: usize,
}

impl Default for IrLimits {
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_nodes: 100_000,
            max_string_bytes: 1024 * 1024,
        }
    }
}

pub fn validate_node(root: &Node, limits: IrLimits) -> Result<(), RenderError> {
    let mut stack = vec![(root, 1_usize)];
    let mut count = 0_usize;
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if count > limits.max_nodes || depth > limits.max_depth {
            return Err(RenderError::new("component IR exceeds structural limits"));
        }
        let check_string = |value: &str| {
            (value.len() <= limits.max_string_bytes)
                .then_some(())
                .ok_or_else(|| RenderError::new("component IR string exceeds byte limit"))
        };
        match node {
            Node::Text(value) => check_string(value)?,
            Node::Element(element) => {
                if !valid_identifier(&element.tag) {
                    return Err(RenderError::new("invalid intrinsic tag"));
                }
                validate_props(&element.props, limits.max_string_bytes)?;
                stack.extend(element.children.iter().map(|child| (child, depth + 1)));
            }
            Node::ClientReference(reference) => {
                check_string(&reference.module_id)?;
                check_string(&reference.export_name)?;
                validate_props(&reference.props, limits.max_string_bytes)?;
                stack.extend(reference.children.iter().map(|child| (child, depth + 1)));
            }
            Node::Fragment(children) | Node::ReactFragment { children, .. } => {
                stack.extend(children.iter().map(|child| (child, depth + 1)));
            }
            Node::Suspense { fallback, content } => {
                stack.push((fallback, depth + 1));
                stack.push((content, depth + 1));
            }
            Node::Null | Node::Slot(_) => {}
        }
    }
    Ok(())
}

fn validate_props(
    props: &BTreeMap<String, PropValue>,
    max_string_bytes: usize,
) -> Result<(), RenderError> {
    for (name, value) in props {
        if !valid_identifier(name) {
            return Err(RenderError::new("invalid component prop name"));
        }
        match value {
            PropValue::String(value) if value.len() > max_string_bytes => {
                return Err(RenderError::new("component prop exceeds byte limit"));
            }
            PropValue::Strings(values)
                if values.iter().any(|value| value.len() > max_string_bytes) =>
            {
                return Err(RenderError::new("component prop exceeds byte limit"));
            }
            PropValue::Node(node) => validate_node(
                node,
                IrLimits {
                    max_string_bytes,
                    ..IrLimits::default()
                },
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Params(BTreeMap<String, ParamValue>);

#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    String(String),
    Strings(Vec<String>),
}

impl Params {
    pub fn insert(&mut self, name: impl Into<String>, value: ParamValue) {
        self.0.insert(name.into(), value);
    }

    pub fn get(&self, name: &str) -> Option<&ParamValue> {
        self.0.get(name)
    }

    pub fn require(&self, name: &str) -> Result<&str, RenderError> {
        match self.get(name) {
            Some(ParamValue::String(value)) => Ok(value),
            Some(ParamValue::Strings(_)) => Err(RenderError::new(format!(
                "route param `{name}` is an array, not a string"
            ))),
            None => Err(RenderError::new(format!("missing route param `{name}`"))),
        }
    }

    pub fn require_strings(&self, name: &str) -> Result<&[String], RenderError> {
        match self.get(name) {
            Some(ParamValue::Strings(values)) => Ok(values),
            Some(ParamValue::String(_)) => Err(RenderError::new(format!(
                "route param `{name}` is a string, not an array"
            ))),
            None => Err(RenderError::new(format!("missing route param `{name}`"))),
        }
    }

    pub fn strings(&self, name: &str) -> Option<&[String]> {
        match self.get(name) {
            Some(ParamValue::Strings(values)) => Some(values),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutProps {
    pub children: Node,
    pub slots: BTreeMap<String, Node>,
    pub params: Params,
    pub request: RequestData,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageProps {
    pub params: Params,
    pub search_params: BTreeMap<String, ParamValue>,
    pub request: RequestData,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequestData {
    pub headers: BTreeMap<String, String>,
    pub cookies: BTreeMap<String, String>,
    pub fetch_responses: BTreeMap<String, HostFetchResponse>,
    pub cache_tags: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFetchResponse {
    pub status: u16,
    pub body: String,
}

impl RequestData {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    pub fn fetch_text(&self, url: &str) -> Result<&str, RenderError> {
        let request = FetchRequest::get(url)?;
        match self.fetch_responses.get(url) {
            Some(response) if (200..300).contains(&response.status) => Ok(&response.body),
            Some(response) => Err(RenderError::new(format!(
                "fetch returned HTTP {}",
                response.status
            ))),
            None => Err(RenderError::HostFetch {
                url: request.url,
                max_response_bytes: request.max_response_bytes,
            }),
        }
    }

    /// Tags the cache entry that owns this Rust component execution.
    pub fn cache_tag(&self, tag: &str) -> Result<(), RenderError> {
        if tag.is_empty() || tag.len() > 256 || tag.chars().any(char::is_control) {
            return Err(RenderError::new("invalid cache tag"));
        }
        if self.cache_tags.contains(tag) {
            Ok(())
        } else {
            Err(RenderError::HostCacheTag {
                tag: tag.to_owned(),
            })
        }
    }
}

/// Revalidation work requested by a Rust Server Action or route handler.
///
/// This deliberately does not live on [`RequestData`]: component render is a
/// read-only phase, and Next.js rejects revalidation from render/cache scopes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevalidationRequest {
    Tag {
        tag: String,
        profile: String,
    },
    UpdateTag {
        tag: String,
    },
    Path {
        path: String,
        kind: Option<RevalidationPathKind>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevalidationPathKind {
    Layout,
    Page,
}

/// Mutation-only host context for future Rust actions and route handlers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MutationContext {
    revalidations: Vec<RevalidationRequest>,
}

impl MutationContext {
    pub fn revalidate_tag(
        &mut self,
        tag: impl Into<String>,
        profile: impl Into<String>,
    ) -> Result<(), RenderError> {
        let tag = tag.into();
        let profile = profile.into();
        validate_revalidation_value("tag", &tag, 256)?;
        validate_revalidation_value("cache profile", &profile, 256)?;
        self.revalidations
            .push(RevalidationRequest::Tag { tag, profile });
        Ok(())
    }

    pub fn update_tag(&mut self, tag: impl Into<String>) -> Result<(), RenderError> {
        let tag = tag.into();
        validate_revalidation_value("tag", &tag, 256)?;
        self.revalidations
            .push(RevalidationRequest::UpdateTag { tag });
        Ok(())
    }

    pub fn revalidate_path(
        &mut self,
        path: impl Into<String>,
        kind: Option<RevalidationPathKind>,
    ) -> Result<(), RenderError> {
        let path = path.into();
        validate_revalidation_value("path", &path, 1024)?;
        if !path.starts_with('/') {
            return Err(RenderError::new("revalidation path must be absolute"));
        }
        self.revalidations
            .push(RevalidationRequest::Path { path, kind });
        Ok(())
    }

    pub fn requests(&self) -> &[RevalidationRequest] {
        &self.revalidations
    }

    pub fn into_requests(self) -> Vec<RevalidationRequest> {
        self.revalidations
    }
}

fn validate_revalidation_value(
    name: &str,
    value: &str,
    max_length: usize,
) -> Result<(), RenderError> {
    if value.is_empty() || value.len() > max_length || value.chars().any(char::is_control) {
        Err(RenderError::new(format!("invalid revalidation {name}")))
    } else {
        Ok(())
    }
}

/// Props for an App Router `error.rs` client boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorProps {
    pub message: String,
    pub digest: Option<String>,
    /// Opaque host control which invokes Next.js' boundary reset callback.
    pub reset: Node,
}

impl ErrorProps {
    pub fn new(message: impl Into<String>, digest: Option<String>, reset: Node) -> Self {
        Self {
            message: message.into(),
            digest,
            reset,
        }
    }
}

impl PageProps {
    pub fn prototype() -> Self {
        Self::default()
    }

    pub fn new(params: Params) -> Self {
        Self {
            params,
            search_params: BTreeMap::new(),
            request: RequestData::default(),
        }
    }

    pub fn with_search_params(params: Params, search_params: BTreeMap<String, ParamValue>) -> Self {
        Self {
            params,
            search_params,
            request: RequestData::default(),
        }
    }

    pub fn with_request(mut self, request: RequestData) -> Self {
        self.request = request;
        self
    }
}

impl LayoutProps {
    pub fn prototype() -> Self {
        Self::new(Node::slot(0), Params::default())
    }

    pub fn new(children: Node, params: Params) -> Self {
        Self {
            children,
            slots: BTreeMap::new(),
            params,
            request: RequestData::default(),
        }
    }

    pub fn with_request(mut self, request: RequestData) -> Self {
        self.request = request;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    Message(String),
    NotFound,
    Redirect {
        location: String,
        status: u16,
    },
    Forbidden,
    Unauthorized,
    HostFetch {
        url: String,
        max_response_bytes: usize,
    },
    HostCacheTag {
        tag: String,
    },
}

impl RenderError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Message(message) => message,
            Self::NotFound => "not found",
            Self::Redirect { .. } => "redirect",
            Self::Forbidden => "forbidden",
            Self::Unauthorized => "unauthorized",
            Self::HostFetch { .. } => "host fetch requested",
            Self::HostCacheTag { .. } => "host cache tag requested",
        }
    }

    pub fn not_found() -> Self {
        Self::NotFound
    }

    pub fn redirect(location: impl Into<String>) -> Self {
        Self::Redirect {
            location: location.into(),
            status: 307,
        }
    }

    pub fn forbidden() -> Self {
        Self::Forbidden
    }

    pub fn unauthorized() -> Self {
        Self::Unauthorized
    }

    pub fn flight_digest(&self) -> String {
        match self {
            Self::Message(_) => "RUST_RSC_RENDER_ERROR".to_owned(),
            Self::NotFound => "NEXT_HTTP_ERROR_FALLBACK;404".to_owned(),
            Self::Redirect { location, status } => {
                format!("NEXT_REDIRECT;replace;{location};{status};")
            }
            Self::Forbidden => "NEXT_HTTP_ERROR_FALLBACK;403".to_owned(),
            Self::Unauthorized => "NEXT_HTTP_ERROR_FALLBACK;401".to_owned(),
            Self::HostFetch { .. } => "RUST_RSC_HOST_FETCH".to_owned(),
            Self::HostCacheTag { .. } => "RUST_RSC_HOST_CACHE_TAG".to_owned(),
        }
    }
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => formatter.write_str(message),
            Self::NotFound => formatter.write_str("not found"),
            Self::Redirect { location, status } => {
                write!(formatter, "redirect to {location} ({status})")
            }
            Self::Forbidden => formatter.write_str("forbidden"),
            Self::Unauthorized => formatter.write_str("unauthorized"),
            Self::HostFetch { url, .. } => write!(formatter, "host fetch requested for {url}"),
            Self::HostCacheTag { tag } => write!(formatter, "host cache tag requested for {tag}"),
        }
    }
}

impl Error for RenderError {}

pub type RenderResult = Result<Node, RenderError>;

/// Encodes a render result into the temporary JSON transport used by the host.
/// The envelope is versioned so incompatible component artifacts fail closed.
pub fn encode_render_result(result: RenderResult) -> String {
    let mut output = String::new();
    write!(output, "{{\"abiVersion\":{ABI_VERSION},\"result\":").unwrap();
    match result {
        Ok(node) => {
            output.push_str("{\"ok\":true,\"node\":");
            push_node(&mut output, node);
            output.push('}');
        }
        Err(error) => {
            output.push_str("{\"ok\":false,\"kind\":");
            let kind = match &error {
                RenderError::Message(_) => "error",
                RenderError::NotFound => "notFound",
                RenderError::Redirect { .. } => "redirect",
                RenderError::Forbidden => "forbidden",
                RenderError::Unauthorized => "unauthorized",
                RenderError::HostFetch { .. } => "hostFetch",
                RenderError::HostCacheTag { .. } => "hostCacheTag",
            };
            push_json_string(&mut output, kind);
            output.push_str(",\"error\":");
            push_json_string(&mut output, error.message());
            if let RenderError::Redirect { location, status } = &error {
                output.push_str(",\"location\":");
                push_json_string(&mut output, &location);
                write!(output, ",\"status\":{status}").unwrap();
            }
            if let RenderError::HostFetch {
                url,
                max_response_bytes,
            } = &error
            {
                output.push_str(",\"url\":");
                push_json_string(&mut output, &url);
                write!(output, ",\"maxResponseBytes\":{max_response_bytes}").unwrap();
            }
            if let RenderError::HostCacheTag { tag } = &error {
                output.push_str(",\"tag\":");
                push_json_string(&mut output, tag);
            }
            output.push('}');
        }
    }
    output.push('}');
    output
}

fn push_node(output: &mut String, node: Node) {
    match node {
        Node::Null => output.push_str("{\"kind\":\"null\"}"),
        Node::Text(value) => {
            output.push_str("{\"kind\":\"text\",\"value\":");
            push_json_string(output, &value);
            output.push('}');
        }
        Node::Slot(SlotId(id)) => {
            write!(output, "{{\"kind\":\"slot\",\"id\":{id}}}").unwrap();
        }
        Node::Element(element) => {
            output.push_str("{\"kind\":\"element\",\"tag\":");
            push_json_string(output, &element.tag);
            output.push_str(",\"props\":{");
            push_props(output, element.props);
            output.push_str("},\"children\":[");
            push_children(output, element.children);
            output.push_str("]}");
        }
        Node::ClientReference(reference) => {
            output.push_str("{\"kind\":\"clientReference\",\"moduleId\":");
            push_json_string(output, &reference.module_id);
            output.push_str(",\"exportName\":");
            push_json_string(output, &reference.export_name);
            output.push_str(",\"chunks\":[");
            for (index, chunk) in reference.chunks.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                push_json_string(output, chunk);
            }
            output.push_str("],\"asynchronous\":");
            output.push_str(if reference.asynchronous {
                "true"
            } else {
                "false"
            });
            output.push_str(",\"props\":{");
            push_props(output, reference.props);
            output.push_str("},\"children\":[");
            push_children(output, reference.children);
            output.push_str("]}");
        }
        Node::Fragment(children) => {
            output.push_str("{\"kind\":\"fragment\",\"children\":[");
            push_children(output, children);
            output.push_str("]}");
        }
        Node::ReactFragment { children, .. } => {
            // The temporary bridge ABI has no keyed-fragment distinction.
            output.push_str("{\"kind\":\"fragment\",\"children\":[");
            push_children(output, children);
            output.push_str("]}");
        }
        Node::Suspense { fallback, content } => {
            output.push_str("{\"kind\":\"suspense\",\"fallback\":");
            push_node(output, *fallback);
            output.push_str(",\"content\":");
            push_node(output, *content);
            output.push('}');
        }
    }
}

fn push_props(output: &mut String, props: BTreeMap<String, PropValue>) {
    let mut first = true;
    for (name, value) in props {
        if !first {
            output.push(',');
        }
        first = false;
        push_json_string(output, &name);
        output.push(':');
        push_prop_value(output, value);
    }
}

fn push_prop_value(output: &mut String, value: PropValue) {
    match value {
        PropValue::Null => output.push_str("null"),
        PropValue::Undefined => output.push_str("null"),
        PropValue::Bool(value) => output.push_str(if value { "true" } else { "false" }),
        PropValue::Number(value) if value.is_finite() => write!(output, "{value}").unwrap(),
        PropValue::Number(_) => output.push_str("null"),
        PropValue::String(value) => push_json_string(output, &value),
        PropValue::Strings(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                push_json_string(output, value);
            }
            output.push(']');
        }
        PropValue::Style(values) => {
            output.push('{');
            for (index, (name, value)) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                push_json_string(output, name);
                output.push(':');
                push_json_string(output, value);
            }
            output.push('}');
        }
        PropValue::Node(node) => push_node(output, *node),
    }
}

fn push_children(output: &mut String, children: Vec<Node>) {
    for (index, child) in children.into_iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_node(output, child);
    }
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch <= '\u{1f}' => write!(output, "\\u{:04x}", ch as u32).unwrap(),
            ch => output.push(ch),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::{
        ABI_VERSION, FetchRequest, IrLimits, LayoutProps, MutationContext, Node, RenderError,
        RequestData, RevalidationPathKind, RevalidationRequest, SlotId, element,
        encode_render_result, validate_node,
    };

    #[test]
    fn encodes_versioned_tree_and_slot() {
        let node = element(
            "main",
            [Node::text("rust"), LayoutProps::prototype().children],
        )
        .prop("className", "shell");
        assert_eq!(
            encode_render_result(Ok(node)),
            format!(
                "{{\"abiVersion\":{ABI_VERSION},\"result\":{{\"ok\":true,\"node\":{{\"kind\":\"\
                 element\",\"tag\":\"main\",\"props\":{{\"className\":\"shell\"}},\"children\":\
                 [{{\"kind\":\"text\",\"value\":\"rust\"}},{{\"kind\":\"slot\",\"id\":0}}]}}}}}}"
            )
        );
    }

    #[test]
    fn request_data_normalizes_header_lookup_and_exposes_cookies() {
        let mut request = RequestData::default();
        request.headers.insert("x-example".into(), "rust".into());
        request.cookies.insert("session".into(), "trusted".into());
        assert_eq!(request.header("X-Example"), Some("rust"));
        assert_eq!(request.cookie("session"), Some("trusted"));
        assert_eq!(request.header("x-matched-path"), None);
    }

    #[test]
    fn revalidation_is_recorded_only_on_the_mutation_context() {
        let mut context = MutationContext::default();
        context.revalidate_tag("products", "max").unwrap();
        context.update_tag("cart").unwrap();
        context
            .revalidate_path("/catalog", Some(RevalidationPathKind::Page))
            .unwrap();
        assert_eq!(
            context.requests(),
            &[
                RevalidationRequest::Tag {
                    tag: "products".into(),
                    profile: "max".into(),
                },
                RevalidationRequest::UpdateTag { tag: "cart".into() },
                RevalidationRequest::Path {
                    path: "/catalog".into(),
                    kind: Some(RevalidationPathKind::Page),
                },
            ]
        );
        assert!(context.update_tag("").is_err());
        assert!(context.revalidate_path("relative", None).is_err());
    }

    #[test]
    fn fetch_requests_reject_unsafe_urls_and_headers() {
        assert!(FetchRequest::get("/relative").is_err());
        assert!(FetchRequest::get("https://catalog.test/items\nsecret").is_err());
        let request = FetchRequest::get("http://127.0.0.1:3041/products").unwrap();
        assert!(request.clone().header("bad header", "value").is_err());
        assert!(
            request
                .clone()
                .header("authorization", "safe\r\ninjected")
                .is_err()
        );
        assert_eq!(
            request
                .header("accept", "application/json")
                .unwrap()
                .headers["accept"],
            "application/json"
        );
    }

    #[test]
    fn validates_props_slots_errors_and_limits() {
        assert_eq!(Node::slot(9), Node::Slot(SlotId(9)));
        assert!(validate_node(&element("div", []), IrLimits::default()).is_ok());
        assert!(validate_node(&element("bad tag", []), IrLimits::default()).is_err());
        assert!(
            validate_node(
                &element("div", [element("span", [Node::text("deep")])]),
                IrLimits {
                    max_depth: 2,
                    ..IrLimits::default()
                },
            )
            .is_err()
        );
        let encoded = encode_render_result(Err(RenderError::redirect("/login")));
        assert!(encoded.contains("\"kind\":\"redirect\""));
        assert!(encoded.contains("\"status\":307"));
    }
}
