#[macro_use]
mod common;

use axum::Router;
use common::build_app;
#[cfg(all(test, feature = "axum-core", feature = "redis-store"))]
use tower_sessions::{fred::prelude::*, RedisStore, SessionManagerLayer};
use tower_sessions::{CookieConfig, SessionManager};

#[cfg(all(test, feature = "axum-core", feature = "redis-store"))]
async fn app(max_age: Option<Duration>) -> Router {
    let database_url = std::option_env!("REDIS_URL").unwrap();

    let config = RedisConfig::from_url(database_url).unwrap();
    let client = RedisClient::new(config, None, None);

    client.connect();
    client.wait_for_connect().await.unwrap();

    let session_store = RedisStore::new(client);
    let session_manager = SessionManager::new(session_store, CookieConfig::default());
    let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

    build_app(session_service, max_age)
}

#[cfg(all(test, feature = "axum-core", feature = "redis-store"))]
route_tests!(app);
