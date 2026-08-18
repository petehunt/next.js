//! A plain Rust endpoint (spec §17). Rust owns status, headers and body.

use next_rs::prelude::*;

pub async fn GET(_req: Request) -> Result<Response> {
    Ok(Json(serde_json::json!([{ "id": 1, "name": "ada" }])).into_response())
}

pub async fn POST(mut req: Request) -> Result<Response> {
    let body: serde_json::Value = req.json(64 * 1024).await?;
    Ok(Json(body).into_response().with_status(StatusCode::CREATED))
}
