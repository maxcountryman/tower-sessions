#[macro_use]
mod common;

use axum::Router;
use common::build_app;
#[cfg(all(test, feature = "axum-core", feature = "mongodb-store"))]
use tower_sessions::{MongoDBStore, SessionManagerLayer};

#[cfg(all(test, feature = "axum-core", feature = "mongodb-store"))]
async fn app(max_age: Option<Duration>) -> Router {
    let database_url = std::option_env!("MONGODB_URL").unwrap();
    let client = mongodb::Client::with_uri_str(database_url).await.unwrap();

    let session_store = MongoDBStore::new(client, "tower-sessions".to_string());
    let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

    build_app(session_manager, max_age)
}

#[cfg(all(test, feature = "axum-core", feature = "mongodb-store"))]
route_tests!(app);
