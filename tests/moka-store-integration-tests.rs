#[macro_use]
mod common;

use axum::Router;
use common::build_app;
#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
use tower_sessions::{MokaStore, SessionManagerLayer};

#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
async fn app(max_age: Option<Duration>) -> Router {
    let moka_store = MokaStore::new(None);
    let session_manager = SessionManagerLayer::new(moka_store).with_secure(true);
    build_app(session_manager, max_age)
}

#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
route_tests!(app);
