use async_trait::async_trait;
pub use fred;
use fred::{
    prelude::{KeysInterface, RedisClient},
    types::Expiration,
};
use time::OffsetDateTime;
use tower_sessions_core::{session::Id, Session, SessionStore};

/// An error type for `RedisStore`.
#[derive(thiserror::Error, Debug)]
pub enum RedisStoreError {
    /// A variant to map to `fred::error::RedisError` errors.
    #[error("Redis error: {0}")]
    Redis(#[from] fred::error::RedisError),

    /// A variant to map `rmp_serde` encode errors.
    #[error("Rust MsgPack encode error: {0}")]
    RmpSerdeEncode(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error("Rust MsgPack decode error: {0}")]
    RmpSerdeDecode(#[from] rmp_serde::decode::Error),
}

/// A Redis session store.
#[derive(Debug, Clone, Default)]
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
    /// let client = RedisClient::default();
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

    async fn save(&self, session: &Session) -> Result<(), Self::Error> {
        let expire = Some(Expiration::EXAT(OffsetDateTime::unix_timestamp(
            session.expiry_date(),
        )));

        self.client
            .set(
                session.id().to_string(),
                rmp_serde::to_vec(&session)?.as_slice(),
                expire,
                None,
                false,
            )
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Session>, Self::Error> {
        let data = self
            .client
            .get::<Option<Vec<u8>>, _>(session_id.to_string())
            .await?;

        if let Some(data) = data {
            Ok(Some(rmp_serde::from_slice(&data)?))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &Id) -> Result<(), Self::Error> {
        self.client.del(session_id.to_string()).await?;
        Ok(())
    }
}
