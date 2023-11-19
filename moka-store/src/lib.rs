use std::convert::Infallible;

use async_trait::async_trait;
use moka::future::Cache;
use time::OffsetDateTime;
use tower_sessions_core::{
    session::{Id, Record},
    SessionStore,
};

/// A session store that uses Moka, a fast and concurrent caching library.
#[derive(Debug, Clone)]
pub struct MokaStore {
    cache: Cache<Id, (Record, OffsetDateTime)>,
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

    async fn save(&self, record: &Record) -> Result<(), Self::Error> {
        self.cache
            .insert(record.id, (record.clone(), record.expiry_date))
            .await;
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Record>, Self::Error> {
        Ok(self
            .cache
            .get(session_id)
            .await
            .filter(|(_, expiry_date)| is_active(*expiry_date))
            .map(|(session, _)| session))
    }

    async fn delete(&self, session_id: &Id) -> Result<(), Self::Error> {
        self.cache.invalidate(session_id).await;
        Ok(())
    }
}

// TODO: Moka supports expiry natively, but that interface is being overhauled
// such that it's more accessible. When that work is done, we should replace
// this with actual expiry.
fn is_active(expiry_date: OffsetDateTime) -> bool {
    expiry_date > OffsetDateTime::now_utc()
}
