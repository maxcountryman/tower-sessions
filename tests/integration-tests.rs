#[macro_use]
mod common;

#[cfg(all(test, feature = "axum-core", feature = "memory-store"))]
mod memory_store_tests {
    use axum::Router;
    use tower_sessions::{MemoryStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let session_store = MemoryStore::default();
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);
        build_app(session_manager, max_age)
    }

    route_tests!(app);
}
