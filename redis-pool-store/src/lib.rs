use async_trait::async_trait;
pub use redis;
pub use redis_pool;
use redis_pool::SingleRedisPool;
use time::OffsetDateTime;
use tower_sessions_core::{session::Id, Session, SessionStore};

/// An error type for `RedisPoolStore`.
#[derive(thiserror::Error, Debug)]
pub enum RedisStoreError {
    /// A variant to map to `redis_pool::errors::RedisPoolError` errors.
    #[error("RedisPool error: {0}")]
    RedisPool(#[from] redis_pool::errors::RedisPoolError),

    /// A variant to map to `redis::RedisError` errors.
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    /// A variant to map `rmp_serde` encode errors.
    #[error("Rust MsgPack encode error: {0}")]
    RmpSerdeEncode(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error("Rust MsgPack decode error: {0}")]
    RmpSerdeDecode(#[from] rmp_serde::decode::Error),
}

/// A Redis session store.
#[derive(Clone)]
pub struct RedisPoolStore {
    client: SingleRedisPool,
}

impl RedisPoolStore {
    /// Create a new Redis store with the provided client.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use redis_pool::RedisPool;
    /// use redis::Client;
    /// use tower_sessions::RedisPoolStore;
    ///
    /// # tokio_test::block_on(async {
    /// let redis_url = "redis://127.0.0.1:6379";
    /// let client = redis::Client::open(redis_url).expect("Error while trying to open the redis connection");
    ///
    /// let session_store = RedisPoolStore::new(client.into());
    /// })
    /// ```
    pub fn new(client: SingleRedisPool) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SessionStore for RedisPoolStore {
    type Error = RedisStoreError;

    async fn save(&self, session: &Session) -> Result<(), Self::Error> {
        let expire = OffsetDateTime::unix_timestamp(session.expiry_date());
        let mut con = self.client.aquire().await?;
        redis::pipe()
            .atomic() //makes this a transation.
            .set(session.id().to_string(), rmp_serde::to_vec(&session)?)
            .ignore()
            .expire_at(session.id().to_string(), expire as usize)
            .ignore()
            .query_async(&mut con)
            .await?;
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Session>, Self::Error> {
        let mut con = self.client.aquire().await?;
        let data: Option<Vec<u8>> = redis::cmd("GET")
            .arg(session_id.to_string())
            .query_async(&mut con)
            .await?;
        if let Some(data) = data {
            Ok(Some(rmp_serde::from_slice(&data)?))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &Id) -> Result<(), Self::Error> {
        let mut con = self.client.aquire().await?;
        redis::pipe()
            .cmd("DEL")
            .arg(session_id.to_string().as_str())
            .query_async(&mut con)
            .await?;
        Ok(())
    }
}
