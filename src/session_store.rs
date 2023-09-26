//! An arbitrary store which houses the session data.

use async_trait::async_trait;

use crate::session::{Session, SessionId, SessionRecord};

/// An arbitrary store which houses the session data.
#[async_trait]
pub trait SessionStore: Clone + Send + Sync + 'static {
    /// An error that occurs when interacting with the store.
    type Error: std::error::Error + Send + Sync;

    /// A method for saving a session in a store.
    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error>;

    /// A method for loading a session from a store.
    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error>;

    /// A method for deleting a session from a store.
    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error>;
}

/// An enumeration of both `SessionStore` error types.
#[derive(thiserror::Error, Debug)]
pub enum CachingStoreError<Cache: SessionStore, Store: SessionStore> {
    /// A cache-related error.
    #[error(transparent)]
    Cache(Cache::Error),

    /// A store-related error.
    #[error(transparent)]
    Store(Store::Error),
}

/// TODO.
#[derive(Debug, Clone)]
pub struct CachingSessionStore<Cache: SessionStore, Store: SessionStore> {
    cache: Cache,
    store: Store,
}

impl<Cache: SessionStore, Store: SessionStore> CachingSessionStore<Cache, Store> {
    /// Create a new `CachingSessionStore`.
    pub fn new(cache: Cache, store: Store) -> Self {
        Self { cache, store }
    }
}

#[async_trait]
impl<Cache, Store> SessionStore for CachingSessionStore<Cache, Store>
where
    Cache: SessionStore + std::fmt::Debug, // TODO: Why is this required to be Debug?
    Store: SessionStore + std::fmt::Debug,
{
    type Error = CachingStoreError<Cache, Store>;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        self.store
            .save(session_record)
            .await
            .map_err(Self::Error::Store)?;
        self.cache
            .save(session_record)
            .await
            .map_err(Self::Error::Cache)?;
        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let session = match self.cache.load(session_id).await {
            Ok(Some(session)) => Some(session),

            Ok(None) => {
                let session = self
                    .store
                    .load(session_id)
                    .await
                    .map_err(Self::Error::Store)?;
                if let Some(session) = session.clone() {
                    let session_record = (&session).into();
                    self.cache
                        .save(&session_record)
                        .await
                        .map_err(Self::Error::Cache)?;
                }
                session
            }

            Err(err) => return Err(Self::Error::Cache(err)),
        };

        Ok(session)
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.store
            .delete(session_id)
            .await
            .map_err(Self::Error::Store)?;
        self.cache
            .delete(session_id)
            .await
            .map_err(Self::Error::Cache)?;
        Ok(())
    }
}
