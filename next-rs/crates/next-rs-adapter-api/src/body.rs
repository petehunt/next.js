use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, Bytes};
use futures_util::stream::Stream;
use next_rs_core::{Body, BodyStream, Error, Result};

/// A `next-rs` body seen as an [`http_body::Body`].
///
/// This is the outbound half of the adapter boundary: everything in the Rust HTTP
/// ecosystem — Axum, Tower, hyper, Actix's `http` interop — speaks `http_body`, so
/// one conversion covers them all (spec §19, §71).
pub struct NextRsBody {
    stream: BodyStream,
    finished: bool,
}

impl NextRsBody {
    pub fn new(body: Body) -> Self {
        Self {
            stream: body.into_stream(),
            finished: false,
        }
    }
}

impl fmt::Debug for NextRsBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NextRsBody")
            .field("finished", &self.finished)
            .finish()
    }
}

impl From<Body> for NextRsBody {
    fn from(value: Body) -> Self {
        Self::new(value)
    }
}

impl http_body::Body for NextRsBody {
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        match this.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(Ok(http_body::Frame::data(chunk)))),
            Poll::Ready(Some(Err(error))) => {
                this.finished = true;
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                this.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.finished
    }
}

/// An [`http_body::Body`] seen as a `next-rs` body stream.
///
/// Trailers are skipped: `next-rs` bodies are byte streams, and no part of the
/// slot protocol depends on trailers.
struct HttpBodyStream<B> {
    body: B,
    finished: bool,
}

impl<B> Stream for HttpBodyStream<B>
where
    B: http_body::Body + Unpin,
    B::Data: Buf,
    B::Error: fmt::Display,
{
    type Item = Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.finished {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.body).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                    Ok(mut data) => {
                        let len = data.remaining();
                        return Poll::Ready(Some(Ok(data.copy_to_bytes(len))));
                    }
                    // Trailers: keep polling for data.
                    Err(_) => continue,
                },
                Poll::Ready(Some(Err(error))) => {
                    this.finished = true;
                    return Poll::Ready(Some(Err(Error::bad_request(format!(
                        "request body failed: {error}"
                    )))));
                }
                Poll::Ready(None) => {
                    this.finished = true;
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Converts any `http_body::Body` into a `next-rs` [`Body`].
pub fn from_http_body<B>(body: B) -> Body
where
    B: http_body::Body + Unpin + Send + 'static,
    B::Data: Buf,
    B::Error: fmt::Display,
{
    Body::from_stream(HttpBodyStream {
        body,
        finished: false,
    })
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    /// A tiny `http_body::Body` yielding fixed chunks.
    struct Chunks(Vec<&'static str>);

    impl http_body::Body for Chunks {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<std::result::Result<http_body::Frame<Bytes>, Self::Error>>> {
            if self.0.is_empty() {
                return Poll::Ready(None);
            }
            let chunk = self.0.remove(0);
            Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from_static(
                chunk.as_bytes(),
            )))))
        }
    }

    #[tokio::test]
    async fn next_rs_body_becomes_an_http_body() {
        use http_body::Body as _;

        let mut body = NextRsBody::new(Body::from_chunks(vec!["ab", "cd"]));
        let mut collected = Vec::new();
        loop {
            let frame =
                futures_util::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
            match frame {
                Some(Ok(frame)) => collected.extend_from_slice(&frame.into_data().unwrap()),
                Some(Err(error)) => panic!("{error}"),
                None => break,
            }
        }
        assert_eq!(collected, b"abcd");
        assert!(body.is_end_stream());
    }

    #[tokio::test]
    async fn an_empty_body_yields_no_frames() {
        use http_body::Body as _;
        let mut body = NextRsBody::new(Body::empty());
        let frame = futures_util::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await;
        assert!(frame.is_none());
    }

    #[tokio::test]
    async fn an_http_body_becomes_a_next_rs_body() {
        let body = from_http_body(Chunks(vec!["hello ", "world"]));
        assert_eq!(body.collect().await.unwrap(), Bytes::from("hello world"));
    }

    #[tokio::test]
    async fn errors_cross_the_boundary_as_bad_requests() {
        struct Failing;

        impl http_body::Body for Failing {
            type Data = Bytes;
            type Error = &'static str;

            fn poll_frame(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<Option<std::result::Result<http_body::Frame<Bytes>, Self::Error>>>
            {
                Poll::Ready(Some(Err("connection reset")))
            }
        }

        let mut stream = from_http_body(Failing).into_stream();
        let error = stream.next().await.unwrap().unwrap_err();
        assert_eq!(error.status().as_u16(), 400);
        assert!(error.message().contains("connection reset"));
    }

    #[tokio::test]
    async fn a_body_that_only_has_trailers_reads_as_empty() {
        struct TrailersOnly(bool);

        impl http_body::Body for TrailersOnly {
            type Data = Bytes;
            type Error = std::convert::Infallible;

            fn poll_frame(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<Option<std::result::Result<http_body::Frame<Bytes>, Self::Error>>>
            {
                if self.0 {
                    return Poll::Ready(None);
                }
                self.0 = true;
                Poll::Ready(Some(Ok(http_body::Frame::trailers(http::HeaderMap::new()))))
            }
        }

        let body = from_http_body(TrailersOnly(false));
        assert!(body.collect().await.unwrap().is_empty());
    }
}
