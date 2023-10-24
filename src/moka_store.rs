use std::convert::Infallible;

use async_trait::async_trait;
use moka::future::Cache;

use crate::{session::SessionId, Session, SessionStore};

/// A session store that uses Moka, a fast and concurrent caching library.
#[derive(Debug, Clone)]
pub struct MokaStore {
    cache: Cache<SessionId, Session>,
}

impl MokaStore {
    /// Create a new Moka store with the provided maximum capacity.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, MokaStore};
    /// let session_store = MokaStore::new(Some(2_000));
    /// ```
    pub fn new(max_capacity: Option<u64>) -> Self {
        // it would be useful to expose more of the CacheBuilder options to the user,
        // but for now this is the most important one
        let cache_builder = match max_capacity {
            Some(capacity) => Cache::builder().max_capacity(capacity),
            None => Cache::builder(),
        };

        Self {
            cache: cache_builder.build(),
        }
    }
}

#[async_trait]
impl SessionStore for MokaStore {
    type Error = Infallible;

    async fn save(&self, session: &Session) -> Result<(), Self::Error> {
        self.cache.insert(*session.id(), session.clone()).await;
        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        Ok(self.cache.get(session_id).await.map(Into::into))
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.cache.invalidate(session_id).await;
        Ok(())
    }
}
