use async_trait::async_trait;
use moka::future::Cache;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

/// A session store that uses Moka, a fast and concurrent caching library.
#[derive(Clone)]
pub struct MokaStore<T: SessionStore> {
    // Note: This stores a Session rather than a SessionRecord because otherwise it can't
    // cache the response when load() is called on the wrapped SessionStore
    cache: Cache<SessionId, Option<Session>>,
    wrapped_store: T,
}

impl<T: SessionStore> MokaStore<T> {
    /// Create a new MokaStore, that acts as a write-trough cache,
    /// using the SessionStore provided as the backing store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_session::{MemoryStore, MokaStore};
    /// let backing_store = MemoryStore::default();
    /// MokaStore::new(backing_store, Some(2000));
    /// ```
    pub fn new(wrapped_store: T, max_capacity: Option<u64>) -> Self {
        // it would be useful to expose more of the CacheBuilder options to the user,
        // but for now this is the most important one
        let cache_builder = match max_capacity {
            Some(capacity) => Cache::builder().max_capacity(capacity),
            None => Cache::builder(),
        };

        Self {
            cache: cache_builder.build(),
            wrapped_store,
        }
    }
}

impl MokaStore<DummyStore> {
    /// Create a new MokaStore, that acts as a in-memory only store, with no
    /// backing store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_session::MokaStore;
    /// MokaStore::new_in_memory();
    /// ```
    pub fn new_in_memory() -> Self {
        Self::new(DummyStore::default(), None)
    }
}

#[async_trait]
impl<T: SessionStore> SessionStore for MokaStore<T> {
    type Error = T::Error;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        self.wrapped_store.save(session_record).await?;

        self.cache
            .insert(session_record.id(), Some(session_record.clone().into()))
            .await;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let session = match self.cache.get(session_id).await {
            Some(session) => session,
            None => {
                let session = self.wrapped_store.load(session_id).await?;
                self.cache.insert(*session_id, session.clone()).await;
                session
            }
        };

        Ok(session)
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.wrapped_store.delete(session_id).await?;
        self.cache.invalidate(session_id).await;
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct DummyStore;

#[async_trait]
impl SessionStore for DummyStore {
    type Error = std::convert::Infallible;

    async fn save(&self, _session_record: &SessionRecord) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn load(&self, _session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        Ok(None)
    }

    async fn delete(&self, _session_id: &SessionId) -> Result<(), Self::Error> {
        Ok(())
    }
}
