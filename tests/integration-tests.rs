#[macro_use]
mod common;

#[cfg(all(test, feature = "axum-core", feature = "memory-store"))]
mod memory_store_tests {
    use axum::Router;
    use tower_sessions::{CookieConfig, MemoryStore, SessionManager, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let session_store = MemoryStore::default();
        let session_manager = SessionManager::new(session_store, CookieConfig::default());
        let session_service = SessionManagerLayer::new(session_manager).with_secure(true);
        build_app(session_service, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
mod moka_store_tests {
    use axum::Router;
    use tower_sessions::{CookieConfig, MokaStore, SessionManager, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let moka_store = MokaStore::new(None);
        let session_manager = SessionManager::new(moka_store, CookieConfig::default());
        let session_service = SessionManagerLayer::new(session_manager).with_secure(true);
        build_app(session_service, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "redis-store"))]
mod redis_store_tests {
    use axum::Router;
    use tower_sessions::{
        fred::prelude::*, CookieConfig, RedisStore, SessionManager, SessionManagerLayer,
    };

    use crate::common::build_app;

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

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "sqlite-store"))]
mod sqlite_store_tests {
    use axum::Router;
    use tower_sessions::{
        sqlx::SqlitePool, CookieConfig, SessionManager, SessionManagerLayer, SqliteStore,
    };

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let session_store = SqliteStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManager::new(session_store, CookieConfig::default());
        let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

        build_app(session_service, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "postgres-store"))]
mod postgres_store_tests {
    use axum::Router;
    use tower_sessions::{
        sqlx::PgPool, CookieConfig, PostgresStore, SessionManager, SessionManagerLayer,
    };

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("POSTGRES_URL").unwrap();
        let pool = PgPool::connect(database_url).await.unwrap();
        let session_store = PostgresStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManager::new(session_store, CookieConfig::default());
        let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

        build_app(session_service, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "mysql-store"))]
mod mysql_store_tests {
    use axum::Router;
    use tower_sessions::{
        sqlx::MySqlPool, CookieConfig, MySqlStore, SessionManager, SessionManagerLayer,
    };

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("MYSQL_URL").unwrap();

        let pool = MySqlPool::connect(database_url).await.unwrap();
        let session_store = MySqlStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManager::new(session_store, CookieConfig::default());
        let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

        build_app(session_service, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "mongodb-store"))]
mod mongodb_store_tests {
    use axum::Router;
    use tower_sessions::{CookieConfig, MongoDBStore, SessionManager, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("MONGODB_URL").unwrap();
        let client = mongodb::Client::with_uri_str(database_url).await.unwrap();
        let session_store = MongoDBStore::new(client, "tower-sessions".to_string());
        let session_manager = SessionManager::new(session_store, CookieConfig::default());
        let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

        build_app(session_service, max_age)
    }

    route_tests!(app);
}

#[cfg(all(
    test,
    feature = "axum-core",
    feature = "sqlite-store",
    feature = "moka-store"
))]
mod caching_store_tests {
    use axum::Router;
    use tower_sessions::{
        sqlx::SqlitePool, CachingSessionStore, CookieConfig, MokaStore, SessionManager,
        SessionManagerLayer, SqliteStore,
    };

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let sqlite_store = SqliteStore::new(pool);
        sqlite_store.migrate().await.unwrap();

        let moka_store = MokaStore::new(None);
        let caching_store = CachingSessionStore::new(moka_store, sqlite_store);

        let session_manager = SessionManager::new(caching_store, CookieConfig::default());
        let session_service = SessionManagerLayer::new(session_manager).with_secure(true);

        build_app(session_service, max_age)
    }

    route_tests!(app);
}
