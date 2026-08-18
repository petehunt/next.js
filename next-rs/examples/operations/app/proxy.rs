//! Request preprocessing in Rust (spec §13).
//!
//! `proxy.rs` runs before route ownership is selected, so continuing is a valid
//! outcome — unlike `route.rs`, which owns the response outright.

use next_rs::prelude::*;

pub async fn proxy(req: &mut Request) -> Result<ProxyResult> {
    if req.path().starts_with("/old") {
        return Ok(Redirect::temporary("/new").into());
    }

    // Rust-only request state that later stages can read (spec §16).
    if let Some(token) = req.headers().get("authorization") {
        let session = Session::authenticated(token.to_owned(), 7);
        req.extensions_mut().insert(session);
    }

    Ok(ProxyResult::Next)
}
