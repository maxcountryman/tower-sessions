use async_trait::async_trait;
use fred::prelude::*;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

/// An error type for `RedisStore`.
#[derive(thiserror::Error, Debug)]
pub enum RedisStoreError {
    /// A variant to map to `fred::error::RedisError` errors.
    #[error("Redis error: {0}")]
    RedisError(#[from] fred::error::RedisError),

    /// A variant to map `serde_json` errors.
    #[error("JSON serialization/deserialization error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
}

/// A Redis session store.
#[derive(Clone, Default)]
pub struct RedisStore {
    client: RedisClient,
}

impl RedisStore {
    /// Create a new Redis store with the provided client.
    pub fn new(client: RedisClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SessionStore for RedisStore {
    type Error = RedisStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        let expiration = session_record
            .expiration_time()
            .map(OffsetDateTime::unix_timestamp)
            .map(Expiration::EXAT);

        self.client
            .set(
                session_record.id().to_string(),
                serde_json::to_string(&session_record)?,
                expiration,
                None,
                false,
            )
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let record_value = self
            .client
            .get::<serde_json::Value, _>(session_id.to_string())
            .await?;

        let session = match record_value {
            serde_json::Value::Null => None,

            record_value => {
                let session_record: SessionRecord = serde_json::from_value(record_value.clone())?;
                Some(session_record.into())
            }
        };

        Ok(session)
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.client.del(session_id.to_string()).await?;
        Ok(())
    }
}
