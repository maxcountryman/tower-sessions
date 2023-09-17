use std::net::SocketAddr;

use axum::{
    error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_sessions::{fred::prelude::*, time::Duration, RedisStore, Session, SessionManagerLayer};

#[derive(Serialize, Deserialize, Default)]
struct Counter(usize);

#[tokio::main]
async fn main() {
    let config = RedisConfig::from_url("redis://127.0.0.1:6379/1").unwrap();
    let client = RedisClient::new(config, None, None);

    let _ = client.connect();
    let _ = client.wait_for_connect().await.unwrap();

    let session_store = RedisStore::new(client);
    let session_service = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: BoxError| async {
            StatusCode::BAD_REQUEST
        }))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_max_age(Duration::seconds(10)),
        );

    let app = Router::new()
        .route("/", get(handler))
        .layer(session_service);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handler(session: Session) -> impl IntoResponse {
    let counter: Counter = session
        .get("counter")
        .expect("Could not deserialize.")
        .unwrap_or_default();

    session
        .insert("counter", counter.0 + 1)
        .expect("Could not serialize.");

    format!("Current count: {}", counter.0)
}
