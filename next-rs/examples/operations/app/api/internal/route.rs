//! A mounted Axum router owns this whole subtree (spec §19, §20).

use axum::{Json, Router, routing::get};

pub fn router() -> Router {
    Router::new().route("/", get(list)).route("/:id", get(show))
}

async fn list() -> Json<serde_json::Value> {
    Json(serde_json::json!([{ "id": 1 }]))
}

async fn show(axum::extract::Path(id): axum::extract::Path<String>) -> String {
    format!("thing {id}")
}
