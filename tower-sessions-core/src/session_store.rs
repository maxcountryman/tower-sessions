//! A session backend for managing session state.
//!
//! This crate provides the ability to use custom backends for session
//! management by implementing the [`SessionStore`] trait. This trait defines
//! the necessary operations for creating, saving, loading, and deleting session
//! records.
//!
//! # Implementing a Custom Store
//!
//! Below is an example of implementing a custom session store using an
//! in-memory [`HashMap`]. This example is for illustration purposes only; you
//! can use the provided [`MemoryStore`] directly without implementing it
//! yourself.
//!
//! ```rust
//! use std::{collections::HashMap, sync::Arc};
//!
//! use time::OffsetDateTime;
//! use tokio::sync::Mutex;
//! use tower_sessions_core::{
//!     session::{Id, Record},
//!     session_store, SessionStore,
//! };
//!
//! #[derive(Clone, Debug, Default)]
//! pub struct MemoryStore(Arc<Mutex<HashMap<Id, Record>>>);
//!
//! #[async_trait]
//! impl SessionStore for MemoryStore {
//!     async fn create(&self, record: &mut Record) -> session_store::Result<()> {
//!         let mut store_guard = self.0.lock().await;
//!         while store_guard.contains_key(&record.id) {
//!             // Session ID collision mitigation.
//!             record.id = Id::default();
//!         }
//!         store_guard.insert(record.id, record.clone());
//!         Ok(())
//!     }
//!
//!     async fn save(&self, record: &Record) -> session_store::Result<()> {
//!         self.0.lock().await.insert(record.id, record.clone());
//!         Ok(())
//!     }
//!
//!     async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
//!         Ok(self
//!             .0
//!             .lock()
//!             .await
//!             .get(session_id)
//!             .filter(|Record { expiry_date, .. }| is_active(*expiry_date))
//!             .cloned())
//!     }
//!
//!     async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
//!         self.0.lock().await.remove(session_id);
//!         Ok(())
//!     }
//! }
//!
//! fn is_active(expiry_date: OffsetDateTime) -> bool {
//!     expiry_date > OffsetDateTime::now_utc()
//! }
//! ```
//!
//! # Session Store Trait
//!
//! The [`SessionStore`] trait defines the interface for session management.
//! Implementations must handle session creation, saving, loading, and deletion.
//!
//! # CachingSessionStore
//!
//! The [`CachingSessionStore`] provides a layered caching mechanism with a
//! cache as the frontend and a store as the backend. This can improve read
//! performance by reducing the need to access the backend store for frequently
//! accessed sessions.
//!
//! # ExpiredDeletion
//!
//! The [`ExpiredDeletion`] trait provides a method for deleting expired
//! sessions. Implementations can optionally provide a method for continuously
//! deleting expired sessions at a specified interval.
use std::{fmt::Debug, future::Future};

use either::Either::{self, Left, Right};
use futures::TryFutureExt;

use crate::{expires::Expires, session::Id};

/// Defines the interface for session management.
///
/// See [`session_store`](crate::session_store) for more details.
// TODO: Remove all `Send` bounds once we have `return_type_notation`:
// https://github.com/rust-lang/rust/issues/109417.
pub trait SessionStore<R: Send + Sync>: Debug + Send + Sync {
    type Error: Send;

    /// Creates a new session in the store with the provided session record.
    ///
    /// Implementers must return an ID in order to avoid ID Collisions. For
    /// example, they might generate a new unique ID or return `Error::Backend`.
    ///
    /// The record is given as an exclusive reference to allow modifications,
    /// such as assigning a new ID, during the creation process.
    fn create(
        &mut self,
        record: &R,
    ) -> impl Future<Output = Result<Id, Self::Error>> + Send;

    /// Saves the provided session record to the store.
    ///
    /// This method is intended for updating the state of an existing session.
    ///
    /// If the session does not exist (`Id` not in store, or expired), then this method should return
    /// `Ok(false)` and should not create the new session. Otherwise it should update the session
    /// and return `Ok(true)`.
    fn save(
        &mut self,
        id: &Id,
        record: &R,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    /// Save the provided session record to the store, and create a new one if it does not exist.
    /// 
    /// ## Caution
    ///
    /// Since the caller can potentially create a new session with a chosen ID, this method should
    /// only be used when it is known that a collision will not occur. The caller should not be in
    /// charge of setting the `Id`, it is rather a job for the `SessionStore` through the `create`
    /// method.
    /// 
    /// This can also accidently increase the lifetime of a session. Suppose a session is loaded
    /// successfully from the store, but then expires before changes are saved. Using this method
    /// will reinstate the session with the same ID, prolonging its lifetime.
    fn save_or_create(
        &mut self,
        id: &Id,
        record: &R,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Loads an existing session record from the store using the provided ID.
    ///
    /// If a session with the given ID exists, it is returned. If the session
    /// does not exist or has been invalidated (e.g., expired), `None` is
    /// returned.
    fn load(
        &mut self,
        id: &Id,
    ) -> impl Future<Output = Result<Option<R>, Self::Error>> + Send;

    /// Deletes a session record from the store using the provided ID.
    ///
    /// If the session existed, it is removed from the store and returns `Ok(true)`,
    /// Otherwise, it returns `Ok(false)`.
    fn delete(&mut self, id: &Id) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    /// Update the ID of a session record.
    ///
    /// This method should return `Ok(None)` if the session does not exist (or is expired).
    /// It should return `Ok(Some(id))` with the new id if it does exist.
    ///
    /// The default implementation uses one `load`, one `create`, and one `delete` operation to
    /// update the `Id`. it is __highly recommended__ to implmement it more efficiently whenever possible.
    fn cycle_id(
        &mut self,
        old_id: &Id,
    ) -> impl Future<Output = Result<Option<Id>, Self::Error>> + Send {
        async move {
            let record = self.load(old_id).await?;
            if let Some(record) = record {
                let new_id = self.create(&record).await?;
                self.delete(old_id).await?;
                Ok(Some(new_id))
            } else {
                Ok(None)
            }
        }
    }
}

/// Provides a layered caching mechanism with a cache as the frontend and a
/// store as the backend.
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
pub struct CachingSessionStore<R, Cache, Store> {
    cache: Cache,
    store: Store,
    phantom: std::marker::PhantomData<R>,
}

impl<R, Cache: Clone, Store: Clone> Clone for CachingSessionStore<R, Cache, Store> {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            store: self.store.clone(),
            phantom: Default::default(),
        }
    }
}

impl<R, Cache: Debug, Store: Debug> Debug for CachingSessionStore<R, Cache, Store> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachingSessionStore")
            .field("cache", &self.cache)
            .field("store", &self.store)
            .field("phantom", &self.phantom)
            .finish()
    }
}

impl<R: Send + Sync, Cache: SessionStore<R>, Store: SessionStore<R>>
    CachingSessionStore<R, Cache, Store>
{
    /// Create a new `CachingSessionStore`.
    pub fn new(cache: Cache, store: Store) -> Self {
        Self {
            cache,
            store,
            phantom: Default::default(),
        }
    }
}

impl<Cache, Store, R> SessionStore<R> for CachingSessionStore<R, Cache, Store>
where
    R: Send + Sync,
    Cache: SessionStore<R>,
    Store: SessionStore<R>,
{
    type Error = Either<Cache::Error, Store::Error>;

    async fn create(&mut self, record: &R) -> Result<Id, Self::Error> {
        let id = self.store.create(record).await.map_err(Right)?;
        self.cache.save_or_create(&id, record).await.map_err(Left)?;
        Ok(id)
    }

    async fn save(&mut self, id: &Id, record: &R) -> Result<bool, Self::Error> {
        let store_save_fut = self.store.save(id, record).map_err(Right);
        let cache_save_fut = self.cache.save(id, record).map_err(Left);

        let (exists_cache, exists_store) = futures::try_join!(cache_save_fut, store_save_fut)?;

        if !exists_store && exists_cache {
            self.cache.delete(id).await.map_err(Left)?;
        }

        Ok(exists_store)
    }

    async fn save_or_create(
            &mut self,
            id: &Id,
            record: &R,
        ) -> Result<(), Self::Error> {
        let store_save_fut = self.store.save_or_create(id, record).map_err(Right);
        let cache_save_fut = self.cache.save_or_create(id, record).map_err(Left);

        futures::try_join!(cache_save_fut, store_save_fut)?;

        Ok(())
    }

    async fn load(&mut self, id: &Id) -> Result<Option<R>, Self::Error> {
        match self.cache.load(id).await {
            // We found a session in the cache, so let's use it.
            Ok(Some(session_record)) => Ok(Some(session_record)),

            // We didn't find a session in the cache, so we'll try loading from the backend.
            //
            // When we find a session in the backend, we'll hydrate our cache with it.
            Ok(None) => {
                let session_record = self.store.load(id).await.map_err(Right)?;

                if let Some(ref session_record) = session_record {
                    self.cache
                        .save(id, session_record)
                        .await
                        .map_err(Either::Left)?;
                }

                Ok(session_record)
            }

            // Some error occurred with our cache so we'll bubble this up.
            Err(err) => Err(Left(err)),
        }
    }

    async fn delete(&mut self, id: &Id) -> Result<bool, Self::Error> {
        let store_delete_fut = self.store.delete(id).map_err(Right);
        let cache_delete_fut = self.cache.delete(id).map_err(Left);

        let (_, in_store) = futures::try_join!(cache_delete_fut, store_delete_fut)?;

        Ok(in_store)
    }

    async fn cycle_id(
            &mut self,
            old_id: &Id,
        ) -> Result<Option<Id>, Self::Error> {
        let delete_cache = self.cache.delete(old_id).map_err(Left);
        let new_id = self.store.cycle_id(old_id).map_err(Right);

        futures::try_join!(delete_cache, new_id).map(|(_, new_id)| new_id)
    }
}

/// Provides a method for deleting expired sessions.
pub trait ExpiredDeletion<T>: SessionStore<T>
where
    Self: Sized,
    T: Expires + Send + Sync,
{
    /// A method for deleting expired sessions from the store.
    fn delete_expired(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;

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
    fn continuously_delete_expired(
        self,
        period: tokio::time::Duration,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let mut interval = tokio::time::interval(period);
        async move {
            interval.tick().await; // The first tick completes immediately; skip.
            loop {
                interval.tick().await;
                self.delete_expired().await?;
            }
        }
    }
}
