use axum::Router;
use common::build_app;
#[cfg(all(test, feature = "axum-core", feature = "memory-store"))]
use tower_sessions::{MemoryStore, SessionManagerLayer};

#[macro_use]
mod common;

async fn app(max_age: Option<Duration>) -> Router {
    let session_store = MemoryStore::default();
    let session_manager = SessionManagerLayer::new(session_store).with_secure(true);
    build_app(session_manager, max_age)
}

route_tests!(app);
