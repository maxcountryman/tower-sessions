use std::net::SocketAddr;

use axum::{
    error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use time::Duration;
use tower::ServiceBuilder;
use tower_sessions::{redis, Expiry, RedisPoolStore, Session, SessionManagerLayer};

const COUNTER_KEY: &str = "counter";

#[derive(Serialize, Deserialize, Default, Debug)]
struct Counter(usize);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let redis_url = "redis://127.0.0.1:6379";
    let client =
        redis::Client::open(redis_url).expect("Error while trying to open the redis connection");

    let session_store = RedisPoolStore::new(client.into());
    let session_service = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: BoxError| async {
            StatusCode::BAD_REQUEST
        }))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_expiry(Expiry::OnInactivity(Duration::seconds(10))),
        );

    let app = Router::new()
        .route("/", get(handler))
        .layer(session_service);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn handler(session: Session) -> impl IntoResponse {
    let counter = session.get(COUNTER_KEY);
    let counter: Counter = counter.expect("Could not deserialize.").unwrap_or_default();
    session
        .insert(COUNTER_KEY, counter.0 + 1)
        .expect("Could not serialize.");

    format!("Current count: {}", counter.0)
}
