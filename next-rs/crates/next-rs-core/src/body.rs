use std::{fmt, pin::Pin};

use bytes::{Bytes, BytesMut};
use futures_util::stream::{self, Stream, StreamExt};

use crate::error::{Error, Result};

/// A boxed body stream.
///
/// Bodies are streams of `Bytes` chunks so that Rust-owned HTML can be produced
/// incrementally (spec §23, §33, §52) without buffering the document.
pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send + 'static>>;

/// A request or response body.
pub enum Body {
    /// No body at all.
    Empty,
    /// A fully buffered body.
    Bytes(Bytes),
    /// A streaming body of unknown or partially known length.
    Stream(BodyStream),
}

impl Body {
    pub fn empty() -> Self {
        Self::Empty
    }

    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        let bytes = bytes.into();
        if bytes.is_empty() {
            Self::Empty
        } else {
            Self::Bytes(bytes)
        }
    }

    /// Wraps a stream of chunks.
    pub fn from_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes>> + Send + 'static,
    {
        Self::Stream(Box::pin(stream))
    }

    /// Wraps a stream of infallible chunks.
    pub fn from_infallible_stream<S, B>(stream: S) -> Self
    where
        S: Stream<Item = B> + Send + 'static,
        B: Into<Bytes>,
    {
        Self::Stream(Box::pin(stream.map(|chunk| Ok(chunk.into()))))
    }

    /// Emits each item in order as its own chunk. Useful in tests and for
    /// synchronous BigPipe-style generators.
    pub fn from_chunks<I, B>(chunks: I) -> Self
    where
        I: IntoIterator<Item = B>,
        I::IntoIter: Send + 'static,
        B: Into<Bytes>,
    {
        Self::Stream(Box::pin(stream::iter(
            chunks.into_iter().map(|chunk| Ok(chunk.into())),
        )))
    }

    /// True only when the body is statically known to be empty. A stream that
    /// happens to yield nothing still reports `false`.
    pub fn is_definitely_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn is_stream(&self) -> bool {
        matches!(self, Self::Stream(_))
    }

    pub fn size_hint(&self) -> SizeHint {
        match self {
            Self::Empty => SizeHint::exact(0),
            Self::Bytes(bytes) => SizeHint::exact(bytes.len() as u64),
            Self::Stream(_) => SizeHint::unknown(),
        }
    }

    /// Converts into a stream of chunks. Empty bodies yield no chunks.
    pub fn into_stream(self) -> BodyStream {
        match self {
            Self::Empty => Box::pin(stream::empty()),
            Self::Bytes(bytes) => Box::pin(stream::once(async move { Ok(bytes) })),
            Self::Stream(stream) => stream,
        }
    }

    /// Buffers the whole body.
    ///
    /// Prefer [`Body::collect_limited`] on request paths; unbounded collection
    /// is only appropriate where the producer is trusted.
    pub async fn collect(self) -> Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Bytes(bytes) => Ok(bytes),
            Self::Stream(mut stream) => {
                let mut buffer = BytesMut::new();
                while let Some(chunk) = stream.next().await {
                    buffer.extend_from_slice(&chunk?);
                }
                Ok(buffer.freeze())
            }
        }
    }

    /// Buffers the body, failing with `PAYLOAD_TOO_LARGE` past `max_bytes`
    /// (spec §93 body limits).
    pub async fn collect_limited(self, max_bytes: usize) -> Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Bytes(bytes) => {
                if bytes.len() > max_bytes {
                    return Err(too_large(max_bytes));
                }
                Ok(bytes)
            }
            Self::Stream(mut stream) => {
                let mut buffer = BytesMut::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    if buffer.len() + chunk.len() > max_bytes {
                        return Err(too_large(max_bytes));
                    }
                    buffer.extend_from_slice(&chunk);
                }
                Ok(buffer.freeze())
            }
        }
    }

    /// Buffers the body and decodes it as UTF-8.
    pub async fn text(self) -> Result<String> {
        let bytes = self.collect().await?;
        String::from_utf8(bytes.to_vec())
            .map_err(|error| Error::bad_request(format!("body is not valid UTF-8: {error}")))
    }

    /// Buffers the body and deserialises it as JSON.
    pub async fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        let bytes = self.collect().await?;
        serde_json::from_slice(&bytes).map_err(Error::from)
    }
}

fn too_large(max_bytes: usize) -> Error {
    Error::payload_too_large(format!("body exceeds the {max_bytes} byte limit"))
}

impl Default for Body {
    fn default() -> Self {
        Self::Empty
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Body::Empty"),
            Self::Bytes(bytes) => f
                .debug_struct("Body::Bytes")
                .field("len", &bytes.len())
                .finish(),
            Self::Stream(_) => f.write_str("Body::Stream"),
        }
    }
}

impl From<()> for Body {
    fn from(_: ()) -> Self {
        Self::Empty
    }
}

impl From<Bytes> for Body {
    fn from(value: Bytes) -> Self {
        Self::from_bytes(value)
    }
}

impl From<Vec<u8>> for Body {
    fn from(value: Vec<u8>) -> Self {
        Self::from_bytes(value)
    }
}

impl From<String> for Body {
    fn from(value: String) -> Self {
        Self::from_bytes(value)
    }
}

impl From<&'static str> for Body {
    fn from(value: &'static str) -> Self {
        Self::from_bytes(Bytes::from_static(value.as_bytes()))
    }
}

/// A lower bound, and optional upper bound, on a body's length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeHint {
    lower: u64,
    upper: Option<u64>,
}

impl SizeHint {
    pub const fn exact(len: u64) -> Self {
        Self {
            lower: len,
            upper: Some(len),
        }
    }

    pub const fn unknown() -> Self {
        Self {
            lower: 0,
            upper: None,
        }
    }

    pub const fn lower(self) -> u64 {
        self.lower
    }

    pub const fn upper(self) -> Option<u64> {
        self.upper
    }

    /// The exact length when known.
    pub const fn exact_len(self) -> Option<u64> {
        match self.upper {
            Some(upper) if upper == self.lower => Some(upper),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_body_collects_to_nothing() {
        let body = Body::empty();
        assert!(body.is_definitely_empty());
        assert_eq!(Body::empty().collect().await.unwrap(), Bytes::new());
    }

    #[tokio::test]
    async fn zero_length_bytes_normalise_to_empty() {
        let body = Body::from_bytes(Vec::new());
        assert!(body.is_definitely_empty());
        assert_eq!(body.size_hint(), SizeHint::exact(0));
    }

    #[tokio::test]
    async fn streams_preserve_chunk_order() {
        let body = Body::from_chunks(vec!["a", "b", "c"]);
        assert!(body.is_stream());
        assert_eq!(body.collect().await.unwrap(), Bytes::from_static(b"abc"));
    }

    #[tokio::test]
    async fn stream_size_hint_is_unknown() {
        let body = Body::from_chunks(vec!["abc"]);
        assert_eq!(body.size_hint(), SizeHint::unknown());
        assert_eq!(SizeHint::unknown().exact_len(), None);
        assert_eq!(SizeHint::exact(3).exact_len(), Some(3));
    }

    #[tokio::test]
    async fn collect_limited_enforces_the_limit() {
        let error = Body::from_bytes("0123456789")
            .collect_limited(4)
            .await
            .unwrap_err();
        assert_eq!(error.status().as_u16(), 413);

        let error = Body::from_chunks(vec!["012", "345"])
            .collect_limited(4)
            .await
            .unwrap_err();
        assert_eq!(error.status().as_u16(), 413);

        assert_eq!(
            Body::from_chunks(vec!["01", "23"])
                .collect_limited(4)
                .await
                .unwrap(),
            Bytes::from_static(b"0123")
        );
    }

    #[tokio::test]
    async fn propagates_stream_errors() {
        let body = Body::from_stream(stream::iter(vec![
            Ok(Bytes::from_static(b"ok")),
            Err(Error::internal("boom")),
        ]));
        assert!(body.collect().await.is_err());
    }

    #[tokio::test]
    async fn decodes_text_and_json() {
        assert_eq!(Body::from("hi").text().await.unwrap(), "hi");

        let value: serde_json::Value = Body::from(r#"{"a":1}"#).json().await.unwrap();
        assert_eq!(value["a"], 1);

        assert!(Body::from_bytes(vec![0xff, 0xfe]).text().await.is_err());
    }
}
