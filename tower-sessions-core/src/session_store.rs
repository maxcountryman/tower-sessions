//! An arbitrary store which houses the session data.

use std::fmt::Debug;

use async_trait::async_trait;
use futures::TryFutureExt;

use crate::session::{Id, Record};

/// An arbitrary store which houses the session data.
///
/// # Implementing your own store
///
/// This crate is designed such that any arbirary session storage backend can be
/// supported simply by implemeting the `SessionStore` trait. While a set of
/// common stores are provided, should those not meet your needs or otherwise we
/// lacking, it is encouraged to implement your own store.
///
/// For example, we might construct a session store for testing purposes that
/// wraps `HashMap`. To do so, we can write a struct that houses this hash map
/// and then implement `SessionStore`.
///
/// ```rust
/// use std::{collections::HashMap, convert::Infallible, sync::Arc};
///
/// use async_trait::async_trait;
/// use parking_lot::Mutex;
/// use tower_sessions::{
///     session::{Id, Record},
///     Session, SessionStore,
/// };
///
/// #[derive(Debug, Clone)]
/// pub struct TestingStore(Arc<Mutex<HashMap<Id, Record>>>);
///
/// #[async_trait]
/// impl SessionStore for TestingStore {
///     type Error = Infallible;
///
///     async fn save(&self, record: &Record) -> Result<(), Self::Error> {
///         self.0.lock().insert(record.id, record.clone());
///         Ok(())
///     }
///
///     async fn load(&self, session_id: &Id) -> Result<Option<Record>, Self::Error> {
///         Ok(self.0.lock().get(session_id).cloned())
///     }
///
///     async fn delete(&self, session_id: &Id) -> Result<(), Self::Error> {
///         self.0.lock().remove(session_id);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait SessionStore: Debug + Clone + Send + Sync + 'static {
    /// An error that occurs when interacting with the store.
    type Error: std::error::Error + Send + Sync;

    /// A method for saving a session in a store.
    async fn save(&self, session_record: &Record) -> Result<(), Self::Error>;

    /// A method for loading a session from a store.
    async fn load(&self, session_id: &Id) -> Result<Option<Record>, Self::Error>;

    /// A method for deleting a session from a store.
    async fn delete(&self, session_id: &Id) -> Result<(), Self::Error>;
}

/// An enumeration of both `SessionStore` error types.
#[derive(thiserror::Error)]
pub enum CachingStoreError<Cache: SessionStore, Store: SessionStore> {
    /// A cache-related error.
    #[error(transparent)]
    Cache(Cache::Error),

    /// A store-related error.
    #[error(transparent)]
    Store(Store::Error),
}

impl<Cache: SessionStore, Store: SessionStore> Debug for CachingStoreError<Cache, Store> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CachingStoreError::Cache(err) => write!(f, "{:?}", err)?,
            CachingStoreError::Store(err) => write!(f, "{:?}", err)?,
        };

        Ok(())
    }
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
    Cache: SessionStore,
    Store: SessionStore,
{
    type Error = CachingStoreError<Cache, Store>;

    async fn save(&self, session: &Record) -> Result<(), Self::Error> {
        let cache_save_fut = self.store.save(session).map_err(Self::Error::Store);
        let store_save_fut = self.cache.save(session).map_err(Self::Error::Cache);

        futures::try_join!(cache_save_fut, store_save_fut)?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Record>, Self::Error> {
        match self.cache.load(session_id).await {
            // We found a session in the cache, so let's use it.
            Ok(Some(session_record)) => Ok(Some(session_record)),

            // We didn't find a session in the cache, so we'll try loading from the backend.
            //
            // When we find a session in the backend, we'll hydrate our cache with it.
            Ok(None) => {
                let session_record = self
                    .store
                    .load(session_id)
                    .await
                    .map_err(Self::Error::Store)?;

                if let Some(ref session_record) = session_record {
                    self.cache
                        .save(session_record)
                        .await
                        .map_err(Self::Error::Cache)?;
                }

                Ok(session_record)
            }

            // Some error occurred with our cache so we'll bubble this up.
            Err(err) => Err(Self::Error::Cache(err)),
        }
    }

    async fn delete(&self, session_id: &Id) -> Result<(), Self::Error> {
        let store_delete_fut = self.store.delete(session_id).map_err(Self::Error::Store);
        let cache_delete_fut = self.cache.delete(session_id).map_err(Self::Error::Cache);

        futures::try_join!(store_delete_fut, cache_delete_fut)?;

        Ok(())
    }
}

/// A trait providing a deletion method for expired methods and optionally a
/// method that runs indefinitely, deleting expired sessions.
#[async_trait]
pub trait ExpiredDeletion: SessionStore {
    /// A method for deleting expired sessions from the store.
    async fn delete_expired(&self) -> Result<(), Self::Error>;

    /// This function will keep running indefinitely, deleting expired rows and
    /// then waiting for the specified period before deleting again.
    ///
    /// Generally this will be used as a task, for example via
    /// `tokio::task::spawn`.
    ///
    /// # Errors
    ///
    /// This function returns a `Result` that contains an error of type
    /// `sqlx::Error` if the deletion operation fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{session_store::ExpiredDeletion, sqlx::SqlitePool, SqliteStore};
    ///
    /// # #[cfg(all(feature = "sqlite-store", feature = "deletion-task"))]
    /// # {
    /// # tokio_test::block_on(async {
    /// let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    /// let session_store = SqliteStore::new(pool);
    ///
    /// tokio::task::spawn(
    ///     session_store
    ///         .clone()
    ///         .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    /// );
    /// # })
    /// # }
    /// ```
    #[cfg(feature = "deletion-task")]
    #[cfg_attr(docsrs, doc(cfg(feature = "deletion-task")))]
    async fn continuously_delete_expired(
        self,
        period: tokio::time::Duration,
    ) -> Result<(), Self::Error> {
        let mut interval = tokio::time::interval(period);
        loop {
            self.delete_expired().await?;
            interval.tick().await;
        }
    }
}
