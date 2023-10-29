use std::net::SocketAddr;

use axum::{
    error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use time::Duration;
use tower::ServiceBuilder;
use tower_sessions::{
    sqlx::PgPool, CachingSessionStore, Expiry, MokaStore, PostgresStore, Session,
    SessionManagerLayer,
};

const COUNTER_KEY: &str = "counter";

#[derive(Serialize, Deserialize, Default)]
struct Counter(usize);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::option_env!("DATABASE_URL").expect("Missing DATABASE_URL.");
    let pool = PgPool::connect(database_url).await?;

    let postgres_store = PostgresStore::new(pool);
    postgres_store.migrate().await?;

    let moka_store = MokaStore::new(Some(2000));
    let caching_store = CachingSessionStore::new(moka_store, postgres_store);

    let session_service = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: BoxError| async {
            StatusCode::BAD_REQUEST
        }))
        .layer(
            SessionManagerLayer::new(caching_store)
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
    let counter: Counter = session
        .get(COUNTER_KEY)
        .expect("Could not deserialize.")
        .unwrap_or_default();

    session
        .insert(COUNTER_KEY, counter.0 + 1)
        .expect("Could not serialize.");

    format!("Current count: {}", counter.0)
}
