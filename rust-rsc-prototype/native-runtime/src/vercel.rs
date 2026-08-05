use std::{
    net::TcpListener,
    sync::{Arc, atomic::AtomicBool},
    thread,
};

use http_body_util::{BodyExt, StreamBody};
use tokio_stream::StreamExt;
use vercel_runtime::{Error, Request, Response, ResponseBody, run as run_runtime, service_fn};

pub(super) async fn run() -> Result<(), Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    thread::Builder::new()
        .name("rust-rsc-vercel-bridge".to_owned())
        .spawn(move || {
            if let Err(error) = super::run_listener(listener, Arc::new(AtomicBool::new(false))) {
                eprintln!("Vercel bridge stopped: {error}");
            }
        })?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    run_runtime(service_fn(move |request| {
        forward(client.clone(), address, request)
    }))
    .await
}

async fn forward(
    client: reqwest::Client,
    address: std::net::SocketAddr,
    request: Request,
) -> Result<Response<ResponseBody>, Error> {
    let (parts, body) = request.into_parts();
    let target = parts
        .uri
        .path_and_query()
        .map_or("/", |value| value.as_str());
    let mut outgoing = client.request(parts.method, format!("http://{address}{target}"));
    for (name, value) in &parts.headers {
        if !is_hop_by_hop(name.as_str()) && name.as_str() != "content-length" {
            outgoing = outgoing.header(name, value);
        }
    }
    let body = body.collect().await?.to_bytes();
    let upstream = outgoing.body(body).send().await?;
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let stream = upstream.bytes_stream().map(|result| {
        result
            .map(hyper::body::Frame::data)
            .map_err(|error| Box::new(error) as Error)
    });
    let mut response = Response::builder().status(status);
    let output_headers = response
        .headers_mut()
        .expect("response builder accepts upstream status");
    for (name, value) in &headers {
        if !is_hop_by_hop(name.as_str()) && name.as_str() != "content-length" {
            output_headers.append(name, value.clone());
        }
    }
    response
        .body(StreamBody::new(stream).into())
        .map_err(Into::into)
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}
