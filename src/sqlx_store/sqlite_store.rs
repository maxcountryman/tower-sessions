use async_trait::async_trait;
use sqlx::sqlite::SqlitePool;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore, SqlxStoreError,
};

/// A SQLite session store.
#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
    table_name: String,
}

impl SqliteStore {
    /// Create a new SQLite store with the provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{sqlx::SqlitePool, SqliteStore};
    ///
    /// # tokio_test::block_on(async {
    /// let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    /// let session_store = SqliteStore::new(pool);
    /// # })
    /// ```
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
                data blob not null
            )
            "#,
            self.table_name
        );
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }

    async fn delete_expired(&self) -> sqlx::Result<()> {
        let query = format!(
            r#"
            delete from {table_name}
            where expiration_time < datetime('now', 'utc')
            "#,
            table_name = self.table_name
        );
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }

    /// This function will keep running indefinitely, deleting expired rows and
    /// then waiting for the specified period before deleting again.
    ///
    /// Generally this will be used as a task, for example via
    /// `tokio::task::spawn`.
    ///
    /// # Arguments
    ///
    /// * `period` - The interval at which expired rows should be deleted.
    ///
    /// # Errors
    ///
    /// This function returns a `Result` that contains an error of type
    /// `sqlx::Error` if the deletion operation fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{sqlx::SqlitePool, SqliteStore};
    ///
    /// # tokio_test::block_on(async {
    /// let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    /// let session_store = SqliteStore::new(pool);
    ///
    /// tokio::task::spawn(
    ///     session_store
    ///         .clone()
    ///         .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    /// );
    /// # })
    /// ```
    #[cfg(feature = "tokio-rt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio-rt")))]
    pub async fn continuously_delete_expired(
        self,
        period: tokio::time::Duration,
    ) -> Result<(), sqlx::Error> {
        let mut interval = tokio::time::interval(period);
        loop {
            self.delete_expired().await?;
            interval.tick().await;
        }
    }
}

#[async_trait]
impl SessionStore for SqliteStore {
    type Error = SqlxStoreError;

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
            .bind(rmp_serde::to_vec(&session_record.data())?)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let query = format!(
            r#"
            select * from {}
            where id = ? and (expiration_time is null or expiration_time > ?)
            "#,
            self.table_name
        );
        let record_value: Option<(String, Option<OffsetDateTime>, Vec<u8>)> =
            sqlx::query_as(&query)
                .bind(session_id.to_string())
                .bind(OffsetDateTime::now_utc())
                .fetch_optional(&self.pool)
                .await?;

        if let Some((session_id, expiration_time, data)) = record_value {
            let session_id = SessionId::try_from(session_id)?;
            let session_record =
                SessionRecord::new(session_id, expiration_time, rmp_serde::from_slice(&data)?);
            Ok(Some(session_record.into()))
        } else {
            Ok(None)
        }
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
