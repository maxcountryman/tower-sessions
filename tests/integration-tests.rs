use axum::Router;
use common::build_app;
use tower_sessions::{CookieConfig, SessionManager};
#[cfg(all(test, feature = "axum-core", feature = "memory-store"))]
use tower_sessions::{MemoryStore, SessionManagerLayer};

#[macro_use]
mod common;

async fn app(max_age: Option<Duration>) -> Router {
    let session_store = MemoryStore::default();
    let session_manager = SessionManager::new(session_store, CookieConfig::default());
    let session_service = SessionManagerLayer::new(session_manager).with_secure(true);
    build_app(session_service, max_age)
}

route_tests!(app);
