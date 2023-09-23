use async_trait::async_trait;
use sqlx::MySqlPool;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore, SqlxStoreError,
};

/// A PostgreSQL session store.
#[derive(Clone, Debug)]
pub struct MySqlStore {
    pool: MySqlPool,
    schema_name: String,
    table_name: String,
}

impl MySqlStore {
    /// Create a new PostgreSQL store with the provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{sqlx::MySqlPool, MySqlStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = MySqlPool::connect(database_url).await.unwrap();
    /// let session_store = MySqlStore::new(pool);
    /// # })
    /// ```
    pub fn new(pool: MySqlPool) -> Self {
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
    /// use tower_sessions::{sqlx::MySqlPool, MySqlStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = MySqlPool::connect(database_url).await.unwrap();
    /// let session_store = MySqlStore::new(pool);
    /// session_store.migrate().await.unwrap();
    /// # })
    /// ```
    pub async fn migrate(&self) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;

        let create_schema_query = format!(
            "create schema if not exists {schema_name}",
            schema_name = self.schema_name,
        );
        sqlx::query(&create_schema_query).execute(&mut *tx).await?;

        let create_table_query = format!(
            r#"
            create table if not exists `{schema_name}`.`{table_name}`
            (
                id char(36) primary key not null,
                expiration_time timestamp null,
                data blob not null
            )
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&create_table_query).execute(&mut *tx).await?;

        tx.commit().await?;

        Ok(())
    }

    #[cfg(feature = "tokio")]
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
    /// use tower_sessions::{sqlx::MySqlPool, MySqlStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = MySqlPool::connect(database_url).await.unwrap();
    /// let session_store = MySqlStore::new(pool);
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
            delete from `{schema_name}`.`{table_name}`
            where expiration_time < utc_timestamp()
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for MySqlStore {
    type Error = SqlxStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            insert into `{schema_name}`.`{table_name}`
              (id, expiration_time, data) values (?, ?, ?)
            on duplicate key update
              expiration_time = values(expiration_time),
              data = values(data)
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
            select * from `{schema_name}`.`{table_name}`
            where id = ?
            and (expiration_time is null or expiration_time > ?)
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
            r#"delete from `{schema_name}`.`{table_name}` where id = ?"#,
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
