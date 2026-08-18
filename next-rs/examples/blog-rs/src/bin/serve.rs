//! `blog-rs` — the native server.
//!
//! The whole deployment is this one process. There is no Node in the request
//! path: `/` and `/posts/:slug` are Rust-owned, and the only React on the page
//! is a Client Component that mounts in the browser (§37, §80).
//!
//! ```bash
//! cargo run --release -p blog-rs            # 127.0.0.1:3001
//! NEXT_RS_PORT=8080 cargo run --release -p blog-rs
//! ```

use std::{net::SocketAddr, sync::Arc};

use next_rs::Result;
use next_rs_runtime_native::NativeServer;

#[tokio::main]
async fn main() -> Result<()> {
    let port: u16 = std::env::var("NEXT_RS_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3001);
    let address = SocketAddr::from(([127, 0, 0, 1], port));

    // One build ID per process. Refresh tokens are bound to it, so a restart
    // invalidates the tokens the previous process minted (§66).
    let build_id = format!("blog-rs-{}", std::process::id());
    let app = Arc::new(blog_rs::app(&build_id, next_rs::crypto::random_key())?);

    // Warm the content cache before the socket opens, so the first request does
    // not pay for parsing three Markdown files.
    let posts = blog_rs::content().all().await?;

    println!(
        "blog-rs: {} post(s), listening on http://{address}",
        posts.len()
    );
    NativeServer::new(app).bind(address).await
}
