use std::{fmt, net::SocketAddr, sync::Arc, time::Duration};

use futures_util::future::BoxFuture;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use next_rs_core::{Error, Result};
use next_rs_html::{ReactRenderer, SsrRequest, SsrResult};
use next_rs_react::SlotError;
use tokio::{net::TcpStream, time::timeout};

use crate::{
    RendererProcess,
    protocol::{ErrorEnvelope, RenderRequest, RenderResponse, RenderSlot},
};

/// Where the renderer lives.
#[derive(Clone)]
pub enum RendererEndpoint {
    /// A renderer this runtime supervises, started on first use (spec §80).
    Process(Arc<RendererProcess>),
    /// A renderer someone else runs — a sidecar container, or a shared pool.
    Fixed(SocketAddr),
}

impl fmt::Debug for RendererEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Process(process) => f.debug_tuple("Process").field(process).finish(),
            Self::Fixed(address) => f.debug_tuple("Fixed").field(address).finish(),
        }
    }
}

/// A [`ReactRenderer`] backed by the renderer process (spec §79).
///
/// One HTTP request per batch: the scheduler already groups slots discovered
/// close together (spec §49), so this crossing happens once for several slots
/// rather than once per slot.
#[derive(Debug, Clone)]
pub struct HttpReactRenderer {
    endpoint: RendererEndpoint,
    build_id: Option<Arc<str>>,
    request_timeout: Duration,
}

impl HttpReactRenderer {
    pub fn new(endpoint: RendererEndpoint) -> Self {
        Self {
            endpoint,
            build_id: None,
            request_timeout: Duration::from_secs(15),
        }
    }

    /// Points at a renderer someone else is running.
    pub fn at(address: SocketAddr) -> Self {
        Self::new(RendererEndpoint::Fixed(address))
    }

    /// Sends the build ID with every batch, so a renderer left over from an
    /// earlier build refuses the work instead of rendering stale components
    /// (spec §66).
    pub fn with_build_id(mut self, build_id: impl AsRef<str>) -> Self {
        self.build_id = Some(Arc::from(build_id.as_ref()));
        self
    }

    pub fn with_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    async fn address(&self) -> Result<SocketAddr> {
        match &self.endpoint {
            RendererEndpoint::Process(process) => process.address().await,
            RendererEndpoint::Fixed(address) => Ok(*address),
        }
    }

    async fn render_batch(&self, requests: Vec<SsrRequest>) -> Result<Vec<SsrResult>> {
        // An empty batch has nothing to render, and asking for the address would
        // start the process. §80 is a hard requirement, so the cheapest way to
        // hold it is to never let a no-op reach the boundary.
        if requests.is_empty() {
            return Ok(Vec::new());
        }

        let address = self.address().await?;
        let payload = serde_json::to_vec(&RenderRequest {
            build_id: self.build_id.as_ref().map(|id| id.to_string()),
            slots: requests
                .iter()
                .map(|request| RenderSlot {
                    slot_id: request.slot_id.to_string(),
                    component_id: request.component_id.clone(),
                    props: request.props.clone(),
                })
                .collect(),
        })
        .map_err(|error| Error::internal(format!("cannot encode a render batch: {error}")))?;

        let response = timeout(self.request_timeout, self.post(address, payload))
            .await
            .map_err(|_| {
                Error::internal(format!(
                    "the React renderer did not answer within {:?}",
                    self.request_timeout
                ))
            })??;

        Ok(pair_results(requests, response))
    }

    async fn post(&self, address: SocketAddr, payload: Vec<u8>) -> Result<RenderResponse> {
        let stream = TcpStream::connect(address).await.map_err(|error| {
            Error::internal(format!(
                "cannot reach the React renderer at {address}: {error}"
            ))
        })?;
        // Nagle would add up to 40ms to a small batch, which is most batches.
        let _ = stream.set_nodelay(true);

        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| {
                Error::internal(format!("React renderer handshake failed: {error}"))
            })?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let request = http::Request::builder()
            .method("POST")
            .uri("/render")
            .header("host", address.to_string())
            .header("content-type", "application/json")
            .body(Full::new(hyper::body::Bytes::from(payload)))
            .map_err(|error| Error::internal(format!("cannot build a render request: {error}")))?;

        let response = sender.send_request(request).await.map_err(|error| {
            Error::internal(format!("the React renderer request failed: {error}"))
        })?;

        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|error| {
                Error::internal(format!("cannot read the React renderer response: {error}"))
            })?
            .to_bytes();

        if !status.is_success() {
            // The renderer's own error envelope is more useful than the status.
            if let Ok(envelope) = serde_json::from_slice::<ErrorEnvelope>(&body) {
                return Err(Error::internal(format!(
                    "the React renderer refused the batch ({}): {}",
                    envelope.error.code, envelope.error.message
                )));
            }
            return Err(Error::internal(format!(
                "the React renderer answered {status}"
            )));
        }

        serde_json::from_slice(&body).map_err(|error| {
            Error::internal(format!(
                "the React renderer returned an unreadable response: {error}"
            ))
        })
    }
}

/// Matches results back to the slots that were asked for.
///
/// The renderer is not trusted to return the same slots, in the same order, or
/// only slots that were asked for: a missing result becomes a slot error rather
/// than a panic or a silently dropped slot.
fn pair_results(requests: Vec<SsrRequest>, response: RenderResponse) -> Vec<SsrResult> {
    requests
        .into_iter()
        .map(|request| {
            let wanted = request.slot_id.to_string();
            let found = response
                .results
                .iter()
                .find(|result| result.slot_id == wanted);

            let outcome = match found {
                Some(result) => match (&result.html, &result.error) {
                    // An error wins over markup: a renderer that reports both is
                    // telling us the markup is not trustworthy.
                    (_, Some(error)) => {
                        Err(SlotError::new(error.code.clone(), error.message.clone()))
                    }
                    (Some(html), None) => Ok(html.clone()),
                    (None, None) => Err(SlotError::new(
                        "NO_SSR_RESULT",
                        "the React renderer returned neither markup nor an error for this slot",
                    )),
                },
                None => Err(SlotError::new(
                    "NO_SSR_RESULT",
                    "the React renderer returned no markup for this slot",
                )),
            };

            SsrResult {
                slot_id: request.slot_id,
                outcome,
            }
        })
        .collect()
}

impl ReactRenderer for HttpReactRenderer {
    fn render(&self, requests: Vec<SsrRequest>) -> BoxFuture<'static, Result<Vec<SsrResult>>> {
        let renderer = self.clone();
        Box::pin(async move { renderer.render_batch(requests).await })
    }
}

#[cfg(test)]
mod tests {
    use next_rs_react::SlotId;
    use serde_json::json;

    use super::*;
    use crate::protocol::{RenderError, RenderResult};

    fn request(slot: &SlotId, component: &str) -> SsrRequest {
        SsrRequest {
            slot_id: slot.clone(),
            component_id: component.to_owned(),
            props: json!({}),
        }
    }

    #[test]
    fn results_are_paired_by_slot_id_not_by_position() {
        let first = SlotId::generate();
        let second = SlotId::generate();
        let requests = vec![request(&first, "A"), request(&second, "B")];

        // Deliberately out of order.
        let response = RenderResponse {
            results: vec![
                RenderResult {
                    slot_id: second.to_string(),
                    html: Some("<b>second</b>".to_owned()),
                    error: None,
                },
                RenderResult {
                    slot_id: first.to_string(),
                    html: Some("<i>first</i>".to_owned()),
                    error: None,
                },
            ],
        };

        let paired = pair_results(requests, response);
        assert_eq!(paired[0].slot_id, first);
        assert_eq!(paired[0].outcome.as_ref().unwrap(), "<i>first</i>");
        assert_eq!(paired[1].slot_id, second);
        assert_eq!(paired[1].outcome.as_ref().unwrap(), "<b>second</b>");
    }

    #[test]
    fn a_missing_result_becomes_a_slot_error() {
        let slot = SlotId::generate();
        let paired = pair_results(
            vec![request(&slot, "A")],
            RenderResponse { results: vec![] },
        );
        assert_eq!(
            paired[0].outcome.as_ref().unwrap_err().code,
            "NO_SSR_RESULT"
        );
    }

    #[test]
    fn an_empty_result_becomes_a_slot_error() {
        let slot = SlotId::generate();
        let paired = pair_results(
            vec![request(&slot, "A")],
            RenderResponse {
                results: vec![RenderResult {
                    slot_id: slot.to_string(),
                    html: None,
                    error: None,
                }],
            },
        );
        assert_eq!(
            paired[0].outcome.as_ref().unwrap_err().code,
            "NO_SSR_RESULT"
        );
    }

    #[test]
    fn an_error_wins_over_markup() {
        let slot = SlotId::generate();
        let paired = pair_results(
            vec![request(&slot, "A")],
            RenderResponse {
                results: vec![RenderResult {
                    slot_id: slot.to_string(),
                    html: Some("<div>partial</div>".to_owned()),
                    error: Some(RenderError {
                        code: "SSR_FAILED".to_owned(),
                        message: "boom".to_owned(),
                    }),
                }],
            },
        );
        let error = paired[0].outcome.as_ref().unwrap_err();
        assert_eq!(error.code, "SSR_FAILED");
        assert_eq!(error.message, "boom");
    }

    #[test]
    fn unexpected_extra_results_are_ignored() {
        let slot = SlotId::generate();
        let paired = pair_results(
            vec![request(&slot, "A")],
            RenderResponse {
                results: vec![
                    RenderResult {
                        slot_id: SlotId::generate().to_string(),
                        html: Some("<div>not ours</div>".to_owned()),
                        error: None,
                    },
                    RenderResult {
                        slot_id: slot.to_string(),
                        html: Some("<div>ours</div>".to_owned()),
                        error: None,
                    },
                ],
            },
        );
        assert_eq!(paired.len(), 1);
        assert_eq!(paired[0].outcome.as_ref().unwrap(), "<div>ours</div>");
    }

    #[tokio::test]
    async fn an_empty_batch_never_reaches_the_process() {
        use crate::{RendererCommand, RendererProcessOptions};

        // The command does not exist, so starting it would fail loudly.
        let process = Arc::new(RendererProcess::new(RendererProcessOptions::new(
            RendererCommand::node("does-not-exist.mjs"),
        )));
        let renderer = HttpReactRenderer::new(RendererEndpoint::Process(Arc::clone(&process)));

        assert!(renderer.render_batch(vec![]).await.unwrap().is_empty());
        // Spec §80: nothing was started.
        assert_eq!(process.spawns(), 0);
    }

    #[tokio::test]
    async fn an_unreachable_renderer_is_reported_clearly() {
        let renderer = HttpReactRenderer::at("127.0.0.1:1".parse().unwrap());
        let error = renderer
            .render_batch(vec![request(&SlotId::generate(), "A")])
            .await
            .unwrap_err();
        assert!(
            error.message().contains("cannot reach the React renderer"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn the_build_id_is_carried_on_the_wire() {
        let renderer = HttpReactRenderer::at("127.0.0.1:9".parse().unwrap()).with_build_id("b-7");
        assert_eq!(renderer.build_id.as_deref(), Some("b-7"));
        let encoded = serde_json::to_string(&RenderRequest {
            build_id: renderer.build_id.as_ref().map(|id| id.to_string()),
            slots: vec![RenderSlot {
                slot_id: "s1".to_owned(),
                component_id: "Metrics".to_owned(),
                props: json!({ "org": 1 }),
            }],
        })
        .unwrap();
        // The JavaScript renderer reads `buildId`, `slotId` and `componentId`.
        assert!(encoded.contains("\"buildId\":\"b-7\""), "{encoded}");
        assert!(encoded.contains("\"slotId\":\"s1\""), "{encoded}");
        assert!(encoded.contains("\"componentId\":\"Metrics\""), "{encoded}");
    }

    #[test]
    fn without_a_build_id_the_field_is_omitted() {
        let encoded = serde_json::to_string(&RenderRequest {
            build_id: None,
            slots: vec![],
        })
        .unwrap();
        assert!(!encoded.contains("build_id"), "{encoded}");
    }
}
