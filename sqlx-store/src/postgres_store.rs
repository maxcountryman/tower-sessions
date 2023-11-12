use async_trait::async_trait;
use sqlx::PgPool;
use time::OffsetDateTime;
use tower_sessions_core::{session::Id, ExpiredDeletion, Session, SessionStore};

use crate::SqlxStoreError;

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
                data bytea not null,
                expiry_date timestamptz not null
            )
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&create_table_query).execute(&mut *tx).await?;

        tx.commit().await?;

        Ok(())
    }
}

#[async_trait]
impl ExpiredDeletion for PostgresStore {
    async fn delete_expired(&self) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            delete from "{schema_name}"."{table_name}"
            where expiry_date < (now() at time zone 'utc')
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

    async fn save(&self, session: &Session) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            insert into "{schema_name}"."{table_name}" (id, data, expiry_date)
            values ($1, $2, $3)
            on conflict (id) do update
            set
              data = excluded.data,
              expiry_date = excluded.expiry_date
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query)
            .bind(&session.id().to_string())
            .bind(rmp_serde::to_vec(&session)?)
            .bind(session.expiry_date())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Session>, Self::Error> {
        let query = format!(
            r#"
            select data from "{schema_name}"."{table_name}"
            where id = $1 and expiry_date > $2
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        let record_value: Option<(Vec<u8>,)> = sqlx::query_as(&query)
            .bind(session_id.to_string())
            .bind(OffsetDateTime::now_utc())
            .fetch_optional(&self.pool)
            .await?;

        if let Some((data,)) = record_value {
            Ok(Some(rmp_serde::from_slice(&data)?))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &Id) -> Result<(), Self::Error> {
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
