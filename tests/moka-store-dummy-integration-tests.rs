#[macro_use]
mod common;

use axum::Router;
use common::build_app;
use tower_sessions::{CookieConfig, SessionManager};
#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
use tower_sessions::{MokaStore, SessionManagerLayer};

#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
async fn app(max_age: Option<Duration>) -> Router {
    let moka_store = MokaStore::new_in_memory();
    let session_manager = SessionManager::new(moka_store, CookieConfig::default());
    let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

    build_app(session_service, max_age)
}

#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
route_tests!(app);
