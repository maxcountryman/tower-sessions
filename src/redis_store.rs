use async_trait::async_trait;
use fred::prelude::*;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

/// An error type for `RedisStore`.
#[allow(clippy::enum_variant_names)]
#[derive(thiserror::Error, Debug)]
pub enum RedisStoreError {
    /// A variant to map to `fred::error::RedisError` errors.
    #[error("Redis error: {0}")]
    RedisError(#[from] fred::error::RedisError),

    /// A variant to map `rmp_serde` encode errors.
    #[error("Rust MsgPack encode error: {0}")]
    RmpSerdeEncodeError(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error("Rust MsgPack decode error: {0}")]
    RmpSerdeDecodeError(#[from] rmp_serde::decode::Error),
}

/// A Redis session store.
#[derive(Clone, Default)]
pub struct RedisStore {
    client: RedisClient,
}

impl RedisStore {
    /// Create a new Redis store with the provided client.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use fred::prelude::*;
    /// use tower_sessions::RedisStore;
    ///
    /// # tokio_test::block_on(async {
    /// let config = RedisConfig::from_url("redis://127.0.0.1:6379/1").unwrap();
    /// let client = RedisClient::new(config, None, None);
    ///
    /// let _ = client.connect();
    /// client.wait_for_connect().await.unwrap();
    ///
    /// let session_store = RedisStore::new(client);
    /// })
    /// ```
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
                rmp_serde::to_vec(&session_record)?.as_slice(),
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
            .get::<Option<Vec<u8>>, _>(session_id.to_string())
            .await?;

        let session = match record_value {
            Some(record_value) => {
                let session_record: SessionRecord = rmp_serde::from_slice(&record_value)?;
                Some(session_record.into())
            }

            None => None,
        };

        Ok(session)
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.client.del(session_id.to_string()).await?;
        Ok(())
    }
}
