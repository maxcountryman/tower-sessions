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
        let pool = RedisPool::new(config, None, None, None, 6).unwrap();

        pool.connect();
        pool.wait_for_connect().await.unwrap();

        let session_store = RedisStore::new(pool);
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

#[cfg(all(test, feature = "axum-core", feature = "mongodb-store"))]
mod mongodb_store_tests {
    use axum::Router;
    use tower_sessions::{mongodb, MongoDBStore, SessionManagerLayer};

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


#[cfg(all(test, feature = "axum-core", feature = "dynamodb-store"))]
mod dynamodb_store_tests {
    use axum::Router;
    use tower_sessions::{
        aws_config,
        aws_sdk_dynamodb,
        DynamoDBStore, DynamoDBStoreProps, DynamoDBStoreKey, SessionManagerLayer
    };
    use crate::common::build_app;

    async fn app(max_age: Option<Duration>) -> Router {
        std::env::set_var("AWS_REGION", "us-east-1");
        std::env::set_var("AWS_ACCESS_KEY_ID", "AKIDLOCALSTACK");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "localstacksecret");

        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region("us-east-1")
            .load()
            .await;

        let dynamodb_local_config = aws_sdk_dynamodb::config::Builder::from(&config)
            .endpoint_url("http://localhost:8000") // 8000 is the default dynamodb port, check test/docker-compose.yml
            .build();

        let client = aws_sdk_dynamodb::Client::from_conf(dynamodb_local_config);
        let store_props = DynamoDBStoreProps {
            table_name: "TowerSessions".to_string(),
            sort_key: Some(DynamoDBStoreKey {
                name: "sort_key".to_string(),
                prefix: Some("TOWER_SESSIONS::".to_string()),
                suffix: None,
            }),
            ..Default::default()
        };

        let mut create_table_request = client.create_table()
            .table_name(&store_props.table_name)
            .attribute_definitions(aws_sdk_dynamodb::types::AttributeDefinition::builder()
                .attribute_name(&store_props.partition_key.name)
                .attribute_type(aws_sdk_dynamodb::types::ScalarAttributeType::S)
                .build()
                .unwrap())
            .key_schema(aws_sdk_dynamodb::types::KeySchemaElement::builder()
                .attribute_name(&store_props.partition_key.name)
                .key_type(aws_sdk_dynamodb::types::KeyType::Hash)
                .build()
                .unwrap())
            .provisioned_throughput(aws_sdk_dynamodb::types::ProvisionedThroughput::builder()
                .read_capacity_units(10)
                .write_capacity_units(5)
                .build()
                .unwrap());

        if let Some(sk) = &store_props.sort_key {
            create_table_request = create_table_request
                .attribute_definitions(aws_sdk_dynamodb::types::AttributeDefinition::builder()
                        .attribute_name(&sk.name)
                        .attribute_type(aws_sdk_dynamodb::types::ScalarAttributeType::S)
                        .build()
                        .unwrap())
                .key_schema(
                    aws_sdk_dynamodb::types::KeySchemaElement::builder()
                        .attribute_name(&sk.name)
                        .key_type(aws_sdk_dynamodb::types::KeyType::Range)
                        .build()
                        .unwrap());
        }

        let _create_table_response = create_table_request.send().await;

        let session_store = DynamoDBStore::new(client, store_props);

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
