//! An arbitrary store which houses the session data.

use std::fmt::Debug;

use async_trait::async_trait;

use crate::session::{Id, Record};

/// Stores must map any errors that might occur during their use to this type.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Encoding failed with: {0}")]
    Encode(String),

    #[error("Decoding failed with: {0}")]
    Decode(String),

    #[error("{0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, Error>;

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
/// use std::{collections::HashMap, sync::Arc};
///
/// use async_trait::async_trait;
/// use parking_lot::Mutex;
/// use tower_sessions::{
///     session::{Id, Record},
///     session_store, Session, SessionStore,
/// };
///
/// #[derive(Debug, Clone)]
/// pub struct TestingStore(Arc<Mutex<HashMap<Id, Record>>>);
///
/// #[async_trait]
/// impl SessionStore for TestingStore {
///     async fn save(&self, record: &Record) -> session_store::Result<()> {
///         self.0.lock().insert(record.id, record.clone());
///         Ok(())
///     }
///
///     async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
///         Ok(self.0.lock().get(session_id).cloned())
///     }
///
///     async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
///         self.0.lock().remove(session_id);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait SessionStore: Debug + Send + Sync + 'static {
    /// Creates a new session in the store with the provided record.
    ///
    /// Implementers must decide how to handle potential ID collisions. For
    /// example, they might generate a new unique ID or return `Error::Backend`.
    ///
    /// The record is given as an exclusive reference to allow modifications,
    /// such as assigning a new ID, during the creation process.
    async fn create(&self, session_record: &mut Record) -> Result<()> {
        default_create(self, session_record).await
    }

    /// Saves the provided session record to the store.
    ///
    /// This method is intended for updating the state of an existing session.
    async fn save(&self, session_record: &Record) -> Result<()>;

    /// Loads an existing session from the store using the provided ID.
    ///
    /// If a session with the given ID exists, it is returned. If the session
    /// does not exist or has been invalidated (e.g., expired), `None` is
    /// returned.
    async fn load(&self, session_id: &Id) -> Result<Option<Record>>;

    /// Deletes a session from the store using the provided ID.
    ///
    /// If the session exists, it is removed from the store.
    async fn delete(&self, session_id: &Id) -> Result<()>;
}

async fn default_create<S: SessionStore + ?Sized>(
    store: &S,
    session_record: &mut Record,
) -> Result<()> {
    tracing::warn!(
        "In order to mitigate potential ID collisions, stores must implement \
         `SessionStore::create` directly. This warning indicates that `SessionStore::save` is \
         being used instead."
    );
    store.save(session_record).await?;
    Ok(())
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
/// ```rust,ignore
/// # tokio_test::block_on(async {
/// use tower_sessions::CachingSessionStore;
/// use tower_sessions_moka_store::MokaStore;
/// use tower_sessions_sqlx_store::{SqlitePool, SqliteStore};
/// let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
/// let sqlite_store = SqliteStore::new(pool);
/// let moka_store = MokaStore::new(Some(2_000));
/// let caching_store = CachingSessionStore::new(moka_store, sqlite_store);
/// # })
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
    async fn create(&self, record: &mut Record) -> Result<()> {
        self.store.create(record).await?;
        self.cache.save(record).await?;
        Ok(())
    }

    async fn save(&self, record: &Record) -> Result<()> {
        let store_save_fut = self.store.save(record);
        let cache_save_fut = self.cache.save(record);

        futures::try_join!(store_save_fut, cache_save_fut)?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Record>> {
        match self.cache.load(session_id).await {
            // We found a session in the cache, so let's use it.
            Ok(Some(session_record)) => Ok(Some(session_record)),

            // We didn't find a session in the cache, so we'll try loading from the backend.
            //
            // When we find a session in the backend, we'll hydrate our cache with it.
            Ok(None) => {
                let session_record = self.store.load(session_id).await?;

                if let Some(ref session_record) = session_record {
                    self.cache.save(session_record).await?;
                }

                Ok(session_record)
            }

            // Some error occurred with our cache so we'll bubble this up.
            Err(err) => Err(err),
        }
    }

    async fn delete(&self, session_id: &Id) -> Result<()> {
        let store_delete_fut = self.store.delete(session_id);
        let cache_delete_fut = self.cache.delete(session_id);

        futures::try_join!(store_delete_fut, cache_delete_fut)?;

        Ok(())
    }
}

/// A trait providing a deletion method for expired methods and optionally a
/// method that runs indefinitely, deleting expired sessions.
#[async_trait]
pub trait ExpiredDeletion: SessionStore
where
    Self: Sized,
{
    /// A method for deleting expired sessions from the store.
    async fn delete_expired(&self) -> Result<()>;

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
    /// ```rust,no_run,ignore
    /// use tower_sessions::session_store::ExpiredDeletion;
    /// use tower_sessions_sqlx_store::{sqlx::SqlitePool, SqliteStore};
    ///
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
    /// ```
    #[cfg(feature = "deletion-task")]
    #[cfg_attr(docsrs, doc(cfg(feature = "deletion-task")))]
    async fn continuously_delete_expired(self, period: tokio::time::Duration) -> Result<()> {
        let mut interval = tokio::time::interval(period);
        loop {
            self.delete_expired().await?;
            interval.tick().await;
        }
    }
}
