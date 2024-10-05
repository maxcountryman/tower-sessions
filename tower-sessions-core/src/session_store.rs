//! A session backend for managing session state.
//!
//! This crate provides the ability to use custom backends for session
//! management by implementing the [`SessionStore`] trait. This trait defines
//! the necessary operations for creating, saving, loading, and deleting session
//! records.
//!
//! # Implementing a Custom Store
//!
//! Every method on the [`SessionStore`] trait describes precisely how it should be implemented.
//! The words _must_ and _should_ are used to indicate the level of necessity for each method.
//! Implementations _must_ adhere to the requirements of the method, while _should_ indicates a
//! recommended approach. These recommendations can be taken more lightly if the implementation is
//! for internal use only.
//!
//! TODO: List good examples of implementations.
//!
//! # CachingSessionStore
//!
//! The [`CachingSessionStore`] provides a layered caching mechanism with a
//! cache as the frontend and a store as the backend. This can improve read
//! performance by reducing the need to access the backend store for frequently
//! accessed sessions.
use std::{fmt::Debug, future::Future};

use either::Either::{self, Left, Right};
use futures_util::TryFutureExt;
use futures_util::future::try_join;

use crate::id::Id;

/// Defines the interface for session management.
///
/// The [`SessionStore::Error`] associated type should be used to represent hard errors that occur
/// during backend operations. For example, an implementation _must not_ return an error if a saved
/// record expired. See each method for more details.
/// __Reasoning__: The [`SessionStore`] should not be responsible for handling logic errors.
/// Methods on this trait are designed to return meaningful results for the caller to handle. The
/// `Err(...)` case is reserved for hard errors that the caller most likely cannot handle, such as
/// network errors, timeouts, invalid backend state/config, etc. These errors usually come from the
/// backend store directly, such as [`sqlx::Error`], [`redis::RedisError`], etc.
///
/// Although recommended, it is not required for a `SessionStore` to handle session expiration. It
/// is acceptable behavior for a session to return a record that is expired. The caller should be
/// the one to decide what storage to use, and to use one that handles expiration if needed.
///
/// [`sqlx::Error`]: https://docs.rs/sqlx
/// [`redis::RedisError`]: https://docs.rs/redis
// TODO: Remove all `Send` bounds once we have `return_type_notation`:
// https://github.com/rust-lang/rust/issues/109417.
pub trait SessionStore<R: Send + Sync>: Send + Sync {
    type Error: Send;

    /// Creates a new session in the store with the provided session record.
    ///
    /// # Implementations
    /// 
    /// In the successful path, Implementations _must_ return a unique ID for the provided record.
    ///
    /// If the a provided record is already expired, the implementation _must_ not return an error.
    /// A correct implementation _should_ instead return a new ID for the record and not insert it
    /// into the store, or it should let the backend store handle the expiration immediately and
    /// return the new ID.
    /// __Reasoning__: Creating a session that is already expired is a logical mistake, not a hard
    /// error. The caller should be responsible for handling this case, when it comes time to
    /// use the session.
    fn create(
        &mut self,
        record: &R,
    ) -> impl Future<Output = Result<Id, Self::Error>> + Send;

    /// Saves the provided session record to the store.
    ///
    /// This method is intended for updating the state of an existing session.
    /// 
    /// # Implementations
    ///
    /// In the successful path, implementations _must_ return `bool` indicating whether the
    /// session existed and thus was updated, or if it did not exist (or was expired) and was not
    /// updated.
    /// __Reasoning__: The caller should be aware of whether the session was successfully updated
    /// or not. If not, then this case can be handled by the caller trivially, thus it is not a
    /// hard error.
    ///
    /// If the implementation handles expiration, id _should_ update the expiration time on the
    /// session record.
    fn save(
        &mut self,
        id: &Id,
        record: &R,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    /// Save the provided session record to the store, and create a new one if it does not exist.
    ///
    /// # Implementations
    ///
    /// In the successful path, implementations _must_ return `Ok(())` if the record was saved or
    /// created with the given ID. This method is only exposed in the API for the sake of other
    /// implementations relying on generic `SessionStore` implementations (see
    /// [`CachingSessionStore`]). End users using `tower-sessions` are not exposed to this method.
    ///
    /// If the implementation handles expiration, id _should_ update the expiration time on the
    /// session record.
    /// 
    /// # Caution
    ///
    /// Since the caller can potentially create a new session with a chosen ID, this method should
    /// only be used by implementations when it is known that a collision will not occur. The caller
    /// should not be in charge of setting the `Id`, it is rather a job for the `SessionStore`
    /// through the `create` method.
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
    /// # Implementations
    ///
    /// If a session with the given ID exists, it is returned as `Some(record)`. If the session
    /// does not exist or has been invalidated (i.e., expired), `None` is returned.
    /// __Reasoning__: Loading a session that does not exist is not a hard error, and the caller
    /// should be responsible for handling this case.
    fn load(
        &mut self,
        id: &Id,
    ) -> impl Future<Output = Result<Option<R>, Self::Error>> + Send;

    /// Deletes a session record from the store using the provided ID.
    ///
    /// # Implementations
    ///
    /// If the session exists (and is not expired), an implmementation _must_ remove the session
    /// from the store and return `Some` with the associated record. Otherwise, it must return
    /// `Ok(None)`.
    /// __Reasoning__: Deleting a session that does not exist is not a hard error, and the caller
    /// should be responsible for handling this case.
    fn delete(&mut self, id: &Id) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    /// Update the ID of a session record.
    ///
    /// # Implementations
    ///
    /// This method _must_ return `Ok(None)` if the session does not exist (or is expired).
    /// It _must_ return `Ok(Some(id))` with the newly assigned id if it does exist.
    /// __Reasoning__: Updating the ID of a session that does not exist is not a hard error, and
    /// the caller should be responsible for handling this case.
    ///
    /// ### Note
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CachingSessionStore<Cache, Store> {
    cache: Cache,
    store: Store,
}

impl<Cache, Store>
    CachingSessionStore<Cache, Store>
{
    /// Create a new `CachingSessionStore`.
    pub fn new(cache: Cache, store: Store) -> Self {
        Self {
            cache,
            store,
        }
    }
}

impl<Cache, Store, R> SessionStore<R> for CachingSessionStore<Cache, Store>
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

        let (exists_cache, exists_store) = try_join(cache_save_fut, store_save_fut).await?;

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

        try_join(cache_save_fut, store_save_fut).await?;

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

        let (_, in_store) = try_join(cache_delete_fut, store_delete_fut).await?;

        Ok(in_store)
    }

    async fn cycle_id(
            &mut self,
            old_id: &Id,
        ) -> Result<Option<Id>, Self::Error> {
        let delete_cache = self.cache.delete(old_id).map_err(Left);
        let new_id = self.store.cycle_id(old_id).map_err(Right);

        try_join(delete_cache, new_id).await.map(|(_, new_id)| new_id)
    }
}
