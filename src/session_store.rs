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

/// A session store for layered caching.
///
/// Contains both a cache, which acts as a frontend, and a store which acts as a
/// backend. Both cache and store implement `SessionStore`.
///
/// By using a cache, the cost of reads can be greatly reduced as once cached,
/// reads need only interact with the frontend, forgoing the cost of retrieving
/// the session record from the backend.
///
/// # Examples
///
/// ```rust
/// # #[cfg(all(feature = "moka_store", feature = "sqlite_store"))]
/// # {
/// # tokio_test::block_on(async {
/// use tower_sessions::{CachingSessionStore, MokaStore, SqlitePool, SqliteStore};
/// let pool = SqlitePool::connect("sqlite::memory:").await?;
/// let sqlite_store = SqliteStore::new(pool);
/// let moka_store = MokaStore::new(Some(2_000));
/// let caching_store = CachingSessionStore::new(moka_store, sqlite_store);
/// # })
/// # }
/// ```
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
        match self.cache.load(session_id).await {
            // We found a session in the cache, so let's use it.
            Ok(Some(session)) => Ok(Some(session).filter(|s| !s.is_empty())),

            // We didn't find a session in the cache, so we'll try loading from the backend.
            //
            // When we find a session in the backend, we'll hydrate our cache with it.
            Ok(None) => {
                let session = self
                    .store
                    .load(session_id)
                    .await
                    .map_err(Self::Error::Store)?;

                if let Some(ref session) = session {
                    let session_record = session.into();
                    self.cache
                        .save(&session_record)
                        .await
                        .map_err(Self::Error::Cache)?;
                } else {
                    // If we know the session doesn't exist in the store, we cache the negative
                    // lookup to avoid future roundtrips to the store.
                    let tombstone = SessionRecord::tombstone_from_id(*session_id);
                    self.cache
                        .save(&tombstone)
                        .await
                        .map_err(Self::Error::Cache)?;
                }

                Ok(session)
            }

            // Some error occurred with our cache so we'll bubble this up.
            Err(err) => Err(Self::Error::Cache(err)),
        }
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
