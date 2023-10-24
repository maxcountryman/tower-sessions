use async_trait::async_trait;
use sqlx::sqlite::SqlitePool;
use time::OffsetDateTime;

use crate::{
    session::SessionId, session_store::ExpiredDeletion, Session, SessionStore, SqlxStoreError,
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
                data blob not null,
                expiry_date integer not null
            )
            "#,
            self.table_name
        );
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl ExpiredDeletion for SqliteStore {
    async fn delete_expired(&self) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            delete from {table_name}
            where expiry_date < datetime('now', 'utc')
            "#,
            table_name = self.table_name
        );
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for SqliteStore {
    type Error = SqlxStoreError;

    async fn save(&self, session: &Session) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            insert into {}
              (id, data, expiry_date) values (?, ?, ?)
            on conflict(id) do update set
              data = excluded.data,
              expiry_date = excluded.expiry_date
            "#,
            self.table_name
        );
        sqlx::query(&query)
            .bind(&session.id().to_string())
            .bind(rmp_serde::to_vec(session)?)
            .bind(session.expiry_date())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let query = format!(
            r#"
            select data from {}
            where id = ? and expiry_date > ?
            "#,
            self.table_name
        );
        let data: Option<(Vec<u8>,)> = sqlx::query_as(&query)
            .bind(session_id.to_string())
            .bind(OffsetDateTime::now_utc())
            .fetch_optional(&self.pool)
            .await?;

        if let Some((data,)) = data {
            Ok(Some(rmp_serde::from_slice(&data)?))
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
