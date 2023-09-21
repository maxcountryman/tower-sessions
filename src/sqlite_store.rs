use async_trait::async_trait;
use sqlx::sqlite::SqlitePool;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

/// An error type for `SqliteStore`.
#[derive(thiserror::Error, Debug)]
pub enum SqliteStoreError {
    /// A variant to map `sqlite` errors.
    #[error("SQLx error: {0}")]
    SqlxError(#[from] sqlx::Error),

    /// A variant to map `serde_json` errors.
    #[error("JSON serialization/deserialization error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
}

/// A SQLite session store.
#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
    table_name: String,
}

impl SqliteStore {
    /// Create a new SQLite store with the provided connection pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            table_name: "tower_sessions".into(),
        }
    }

    /// Set the session table name with the provided name.
    pub fn with_table_name(mut self, table_name: impl AsRef<str>) -> Result<Self, String> {
        let table_name = table_name.as_ref();
        if !is_valid_table_name(table_name) {
            return Err(format!(
                "Invalid table name '{}'. Table names must be alphanumeric and may contain \
                 hyphens or underscores.",
                table_name
            ));
        }

        self.table_name = table_name.to_owned();
        Ok(self)
    }

    /// Migrate the session schema.
    pub async fn migrate(&self) -> sqlx::Result<()> {
        let query = format!(
            r#"
            create table if not exists {}
            (
                id text primary key not null,
                expiration_time integer null,
                data text not null
            )
            "#,
            self.table_name
        );
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for SqliteStore {
    type Error = SqliteStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            insert into {}
              (id, expiration_time, data) values (?, ?, ?)
            on conflict(id) do update set
              expiration_time = excluded.expiration_time,
              data = excluded.data
            "#,
            self.table_name
        );
        sqlx::query(&query)
            .bind(&session_record.id().to_string())
            .bind(session_record.expiration_time())
            .bind(serde_json::to_string(&session_record)?)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let query = format!(
            r#"
            select data from {}
            where id = ? and (expiration_time is null or expiration_time > ?)
            "#,
            self.table_name
        );
        let record_value: Option<String> = sqlx::query_scalar(&query)
            .bind(session_id.to_string())
            .bind(OffsetDateTime::now_utc())
            .fetch_optional(&self.pool)
            .await?;

        Ok(record_value
            .map(|json| serde_json::from_str::<SessionRecord>(&json))
            .transpose()?
            .map(Into::into))
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            delete from {} where id = ?
            "#,
            self.table_name
        );
        sqlx::query(&query)
            .bind(&session_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

fn is_valid_table_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
