use std::net::SocketAddr;

use axum::{
    error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    r2d2::{ConnectionManager, Pool},
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use time::Duration;
use tower::ServiceBuilder;
use tower_sessions::{
    diesel_store::{DieselStore, DieselStoreError, SessionTable},
    session_store::ExpiredDeletion,
    Session, SessionManagerLayer,
};

const COUNTER_KEY: &str = "counter";

#[derive(Serialize, Deserialize, Default)]
struct Counter(usize);

diesel::table! {
    /// The session table used by default by the diesel-store implemenattion
    my_sessions {
        /// `id` column, contains a session id
        id -> Text,
        /// `expiration_time` column, contains an optional expiration timestamp
        expiration_time -> Nullable<Timestamp>,
        /// `data` column, contains serialized session data
        data -> Binary,
    }
}

impl SessionTable<SqliteConnection> for self::my_sessions::table {
    type ExpirationTime = self::my_sessions::expiration_time;

    fn insert(
        conn: &mut SqliteConnection,
        session_record: &tower_sessions::session::SessionRecord,
    ) -> Result<(), DieselStoreError> {
        diesel::insert_into(my_sessions::table)
            .values((
                my_sessions::id.eq(session_record.id().to_string()),
                my_sessions::expiration_time.eq(session_record
                    .expiration_time()
                    .map(|t| time::PrimitiveDateTime::new(t.date(), t.time()))),
                my_sessions::data.eq(rmp_serde::to_vec(&session_record.data())?),
            ))
            .execute(conn)?;
        Ok(())
    }

    fn migrate(conn: &mut SqliteConnection) -> Result<(), DieselStoreError> {
        // or create the table via normal diesel migrations on startup and leave that
        // function empty
        conn.batch_execute(
            "CREATE TABLE `my_sessions` (`id` TEXT PRIMARY KEY NOT NULL, `expiration_time` TEXT \
             NULL, `data` BLOB NOT NULL);",
        )?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = Pool::builder()
        .max_size(1)
        .build(ConnectionManager::<SqliteConnection>::new(":memory:"))
        .unwrap();
    let session_store = DieselStore::with_table(my_sessions::table, pool);
    session_store.migrate().await?;

    let deletion_task = tokio::task::spawn(
        session_store
            .clone()
            .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    );

    let session_service = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: BoxError| async {
            StatusCode::BAD_REQUEST
        }))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_max_age(Duration::seconds(10)),
        );

    let app = Router::new()
        .route("/", get(handler))
        .layer(session_service);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    deletion_task.await??;

    Ok(())
}

async fn handler(session: Session) -> impl IntoResponse {
    let counter: Counter = session
        .get(COUNTER_KEY)
        .expect("Could not deserialize.")
        .unwrap_or_default();

    session
        .insert(COUNTER_KEY, counter.0 + 1)
        .expect("Could not serialize.");

    format!("Current count: {}", counter.0)
}
