#[macro_use]
mod common;

use axum::Router;
use common::build_app;
#[cfg(all(test, feature = "axum-core", feature = "postgres-store"))]
use tower_sessions::{sqlx::PgPool, PostgresStore, SessionManagerLayer};

#[cfg(all(test, feature = "axum-core", feature = "postgres-store"))]
async fn app(max_age: Option<Duration>) -> Router {
    let database_url = std::option_env!("POSTGRES_URL").unwrap();

    let pool = PgPool::connect(database_url).await.unwrap();
    let session_store = PostgresStore::new(pool);
    session_store.migrate().await.unwrap();
    let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

    build_app(session_manager, max_age)
}

#[cfg(all(test, feature = "axum-core", feature = "postgres-store"))]
route_tests!(app);
