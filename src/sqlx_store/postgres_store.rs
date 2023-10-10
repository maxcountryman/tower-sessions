use async_trait::async_trait;
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore, SqlxStoreError,
};

/// A PostgreSQL session store.
#[derive(Clone, Debug)]
pub struct PostgresStore {
    pool: PgPool,
    schema_name: String,
    table_name: String,
}

impl PostgresStore {
    /// Create a new PostgreSQL store with the provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{sqlx::PgPool, PostgresStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = PgPool::connect(database_url).await.unwrap();
    /// let session_store = PostgresStore::new(pool);
    /// # })
    /// ```
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            schema_name: "tower_sessions".to_string(),
            table_name: "session".to_string(),
        }
    }

    /// Migrate the session schema.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{sqlx::PgPool, PostgresStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = PgPool::connect(database_url).await.unwrap();
    /// let session_store = PostgresStore::new(pool);
    /// session_store.migrate().await.unwrap();
    /// # })
    /// ```
    pub async fn migrate(&self) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;

        let create_schema_query = format!(
            r#"create schema if not exists "{schema_name}""#,
            schema_name = self.schema_name,
        );
        // Concurrent create schema may fail due to duplicate key violations.
        //
        // This works around that by assuming the schema must exist on such an error.
        if let Err(err) = sqlx::query(&create_schema_query).execute(&mut *tx).await {
            if !err
                .to_string()
                .contains("duplicate key value violates unique constraint")
            {
                return Err(err);
            }

            return Ok(());
        }

        let create_table_query = format!(
            r#"
            create table if not exists "{schema_name}"."{table_name}"
            (
                id text primary key not null,
                expiration_time timestamptz null,
                data bytea not null
            )
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&create_table_query).execute(&mut *tx).await?;

        tx.commit().await?;

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
    /// use tower_sessions::{sqlx::PgPool, PostgresStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = PgPool::connect(database_url).await.unwrap();
    /// let session_store = PostgresStore::new(pool);
    ///
    /// tokio::task::spawn(
    ///     session_store
    ///         .clone()
    ///         .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    /// );
    /// # })
    /// ```
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

    async fn delete_expired(&self) -> sqlx::Result<()> {
        let query = format!(
            r#"
            delete from "{schema_name}"."{table_name}"
            where expiration_time < (now() at time zone 'utc')
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for PostgresStore {
    type Error = SqlxStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            insert into "{schema_name}"."{table_name}" (id, expiration_time, data)
            values ($1, $2, $3)
            on conflict (id) do update
            set
              expiration_time = excluded.expiration_time,
              data = excluded.data
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
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
            select * from "{schema_name}"."{table_name}"
            where id = $1
            and (expiration_time is null or expiration_time > $2)
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
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
            r#"delete from "{schema_name}"."{table_name}" where id = $1"#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query)
            .bind(&session_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}
