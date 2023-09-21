use async_trait::async_trait;
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore, SqlxStoreError,
};

/// A SQLite session store.
#[derive(Clone, Debug)]
pub struct PostgresStore {
    pool: PgPool,
    schema_name: String,
    table_name: String,
}

impl PostgresStore {
    /// Create a new PostgreSQL store with the provided connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            schema_name: "tower_sessions".to_string(),
            table_name: "session".to_string(),
        }
    }

    /// Migrate the session schema.
    pub async fn migrate(&self) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;

        let create_schema_query = format!(
            "create schema if not exists {schema_name}",
            schema_name = self.schema_name,
        );
        sqlx::query(&create_schema_query).execute(&mut *tx).await?;

        let create_table_query = format!(
            r#"
            create table if not exists {schema_name}.{table_name}
            (
                id text primary key not null,
                expiration_time timestamptz null,
                data text not null
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
impl SessionStore for PostgresStore {
    type Error = SqlxStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            insert into {schema_name}.{table_name}
              (id, expiration_time, data) values ($1, $2, $3)
            on conflict(id) do update set
              expiration_time = excluded.expiration_time,
              data = excluded.data
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query)
            .bind(&session_record.id().to_string())
            .bind(session_record.expiration_time())
            .bind(serde_json::to_string(&session_record.data())?)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let query = format!(
            r#"
            select * from {schema_name}.{table_name}
            where id = $1
            and (expiration_time is null or expiration_time > $2)
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        let record_value: Option<(String, Option<OffsetDateTime>, String)> = sqlx::query_as(&query)
            .bind(session_id.to_string())
            .bind(OffsetDateTime::now_utc())
            .fetch_optional(&self.pool)
            .await?;

        if let Some((session_id, expiration_time, data)) = record_value {
            let session_id = SessionId::try_from(session_id)?;
            let session_record =
                SessionRecord::new(session_id, expiration_time, serde_json::from_str(&data)?);
            Ok(Some(session_record.into()))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        let query = format!(
            r#"
            delete from {schema_name}.{table_name} where id = $1
            "#,
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
