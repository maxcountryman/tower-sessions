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

#[cfg(all(test, feature = "axum-core", feature = "moka-store"))]
mod moka_store_tests {
    use axum::Router;
    use tower_sessions::{MokaStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let moka_store = MokaStore::new(None);
        let session_manager = SessionManagerLayer::new(moka_store).with_secure(true);
        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "redis-store"))]
mod redis_store_tests {
    use axum::Router;
    use tower_sessions::{fred::prelude::*, RedisStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("REDIS_URL").unwrap();

        let config = RedisConfig::from_url(database_url).unwrap();
        let client = RedisClient::new(config, None, None, None);

        client.connect();
        client.wait_for_connect().await.unwrap();

        let session_store = RedisStore::new(client);
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "sqlite-store"))]
mod sqlite_store_tests {
    use axum::Router;
    use tower_sessions::{sqlx::SqlitePool, SessionManagerLayer, SqliteStore};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let session_store = SqliteStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "postgres-store"))]
mod postgres_store_tests {
    use axum::Router;
    use tower_sessions::{sqlx::PgPool, PostgresStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("POSTGRES_URL").unwrap();
        let pool = PgPool::connect(database_url).await.unwrap();
        let session_store = PostgresStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "mysql-store"))]
mod mysql_store_tests {
    use axum::Router;
    use tower_sessions::{sqlx::MySqlPool, MySqlStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("MYSQL_URL").unwrap();

        let pool = MySqlPool::connect(database_url).await.unwrap();
        let session_store = MySqlStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "diesel-store"))]
mod diesel_sqlite_store_tests {
    use axum::Router;
    use diesel::{
        prelude::*,
        r2d2::{ConnectionManager, Pool},
    };
    use tower_sessions::{diesel_store::DieselStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let pool = Pool::builder()
            .max_size(1)
            .build(ConnectionManager::<SqliteConnection>::new(":memory:"))
            .unwrap();
        let session_store = DieselStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(
    test,
    feature = "axum-core",
    feature = "diesel-store",
    feature = "__diesel_postgres"
))]
mod diesel_pg_store_tests {
    use axum::Router;
    use diesel::{
        prelude::*,
        r2d2::{ConnectionManager, Pool},
    };
    use tower_sessions::{diesel_store::DieselStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("POSTGRES_URL").unwrap();
        let pool = Pool::builder()
            .max_size(1)
            .build(ConnectionManager::<PgConnection>::new(database_url))
            .unwrap();
        let session_store = DieselStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(
    test,
    feature = "axum-core",
    feature = "diesel-store",
    feature = "__diesel_mysql"
))]
mod diesel_mysql_store_tests {
    use axum::Router;
    use diesel::{
        prelude::*,
        r2d2::{ConnectionManager, Pool},
    };
    use tower_sessions::{diesel_store::DieselStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("MYSQL_URL").unwrap();
        let pool = Pool::builder()
            .max_size(1)
            .build(ConnectionManager::<MysqlConnection>::new(database_url))
            .unwrap();
        let session_store = DieselStore::new(pool);
        session_store.migrate().await.unwrap();
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}

#[cfg(all(test, feature = "axum-core", feature = "mongodb-store"))]
mod mongodb_store_tests {
    use axum::Router;
    use tower_sessions::{MongoDBStore, SessionManagerLayer};

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let database_url = std::option_env!("MONGODB_URL").unwrap();
        let client = mongodb::Client::with_uri_str(database_url).await.unwrap();
        let session_store = MongoDBStore::new(client, "tower-sessions".to_string());
        let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

        build_app(session_manager, max_age)
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
        sqlx::SqlitePool, CachingSessionStore, MokaStore, SessionManagerLayer, SqliteStore,
    };

    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let sqlite_store = SqliteStore::new(pool);
        sqlite_store.migrate().await.unwrap();

        let moka_store = MokaStore::new(None);
        let caching_store = CachingSessionStore::new(moka_store, sqlite_store);

        let session_manager = SessionManagerLayer::new(caching_store).with_secure(true);

        build_app(session_manager, max_age)
    }

    route_tests!(app);
}
