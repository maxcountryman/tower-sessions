#[macro_use]
mod common;

use axum::Router;
use common::build_app;
#[cfg(all(test, feature = "axum-core", feature = "sqlite-store"))]
use tower_sessions::{sqlx::SqlitePool, SessionManagerLayer, SqliteStore};
use tower_sessions::{CookieConfig, SessionManager};

#[cfg(all(test, feature = "axum-core", feature = "sqlite-store"))]
async fn app(max_age: Option<Duration>) -> Router {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let session_store = SqliteStore::new(pool);
    session_store.migrate().await.unwrap();
    let session_manager = SessionManager::new(session_store, CookieConfig::default());
    let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

    build_app(session_service, max_age)
}

#[cfg(all(test, feature = "axum-core", feature = "sqlite-store"))]
route_tests!(app);
