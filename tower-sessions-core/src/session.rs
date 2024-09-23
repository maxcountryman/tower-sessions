//! A session which allows HTTP applications to associate data with visitors.
use std::{
    fmt::{self, Debug, Display},
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    str,
    sync::{Arc, Mutex},
};
// TODO: Remove send + sync bounds on `R` once return type notation is stable.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};

use crate::SessionStore;

/// A session which allows HTTP applications to associate key-value pairs with
/// visitors.
pub struct LazySession<R, Store> {
    /// This will be `None` if the endpoint has not received a session cookie or if the it could
    /// not be parsed.
    id: Option<Id>,
    store: Store,
    /// Data associated with the session, it is `None` if the session was not loaded yet.
    data: PhantomData<R>,
    updater: Updater,
}

impl<R, Store: Clone> Clone for LazySession<R, Store> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            id: self.id,
            data: PhantomData,
            updater: self.updater.clone(),
        }
    }
}

impl<R, Store: Debug> Debug for LazySession<R, Store> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("store", &self.store)
            .field("id", &self.id)
            .field("data", &self.data)
            .finish()
    }
}

impl<R: Send + Sync, Store: SessionStore<R>> LazySession<R, Store> {
    /// Creates a new session with the session ID, store, and expiry.
    ///
    /// This method is lazy and does not invoke the overhead of talking to the
    /// backing store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// Session::new(None, store, None);
    /// ```
    pub fn new(store: Store, id: Option<Id>, updater: Updater) -> Self {
        Self {
            store,
            id,
            data: Default::default(),
            updater,
        }
    }

    pub async fn load(mut self) -> Result<Option<Session<R, Store>>, Store::Error> {
        Ok(if let Some(id) = self.id {
            let data = self.store.load(&id).await?;
            data.map(|data| Session {
                store: self.store,
                id,
                data,
                updater: self.updater,
            })
        } else {
            None
        })
    }
}

/// A loaded session.
///
/// This struct has a somewhat convoluted API, but it is designed to be nearly impossible to
/// misuse. Luckily, it only has a handful of methods, and each of them document how they work.
pub struct Session<R: Send + Sync, Store: SessionStore<R>> {
    store: Store,
    id: Id,
    data: R,
    updater: Updater,
}

impl<R, Store> Session<R, Store>
where
    R: Send + Sync,
    Store: SessionStore<R>,
{
    /// Read the data associated with the session.
    pub fn data(&self) -> &R {
        &self.data
    }

    /// Mutably access the data associated with the session.
    ///
    /// Returns a [`DataMut`], which functions similarly to a `Guard`.
    pub fn data_mut(self) -> DataMut<R, Store> {
        DataMut { session: self }
    }

    /// Delete the session from the store.
    ///
    /// This method returns a boolean indicating whether the session was deleted from the store.
    /// If the `Store` returns `Ok(false)` if the session simply did not exist. This can happen if
    /// it was deleted by another request or if the session expired between the time it was
    /// loaded and the time this method was called.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    pub async fn delete(mut self) -> Result<bool, Store::Error> {
        let deleted = self.store.delete(&self.id).await?;
        self.updater
            .lock()
            .expect("lock should not be poisoned")
            .replace(SessionUpdate::Delete);
        let _ = ManuallyDrop::new(self);
        Ok(deleted)
    }

    /// Cycle the session ID.
    ///
    /// This consumes the current session and returns a new session with the new ID. This method
    /// should be used to mitigate [session fixation attacks](https://www.acrossecurity.com/papers/session_fixation.pdf).
    ///
    /// This method returns `Ok(None)` if the session was deleted or expired between the time it
    /// was loaded and the time this method was called. Otherwise, it returns the new
    /// `Some(Session)`.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    pub async fn cycle(mut self) -> Result<Option<Session<R, Store>>, Store::Error> {
        if let Some(new_id) = self.store.cycle_id(&self.id).await? {
            self.updater
                .lock()
                .expect("lock should not be poisoned")
                .replace(SessionUpdate::Set(new_id));
            self.id = new_id;
            return Ok(Some(self));
        }
        Ok(None)
    }
}

/// A struct that provides mutable access to a session's data.
/// Access to `R` is provided through `Deref` and `DerefMut`.
///
/// This is created by calling `data_mut` on a `Session`.
/// To retrieve the `Session`, call `save` on this struct.
///
/// Saving is done automatically when this struct is dropped, but errors are ignored when doing so;
/// Hence, it should be done explicitly with `save` whenever possible.
pub struct DataMut<R: Send + Sync, Store: SessionStore<R>> {
    session: Session<R, Store>,
}

impl<R: Send + Sync, Store: SessionStore<R>> DataMut<R, Store> {
    /// Save the session data to the store.
    ///
    /// It is preferred to use this method to save the session rather than through `Drop`.
    ///
    /// This method returns the `Session` if the data was saved successfully. It returns
    /// `Ok(None)` when the session was deleted or expired between the time it was loaded and the
    /// time this method was called.
    ///
    /// # Error
    ///
    /// Errors if the underlying store errors.
    pub async fn save(mut self) -> Result<Option<Session<R, Store>>, Store::Error> {
        if self
            .session
            .store
            .save(&self.session.id, &self.session.data)
            .await?
        {
            let self_ = ManuallyDrop::new(self);
            // Safety: https://internals.rust-lang.org/t/destructuring-droppable-structs/20993/16,
            // we need to destructure the struct but it implements `Drop`.
            Ok(Some(unsafe { std::ptr::read(&self_.session as *const _) }))
        } else {
            let _ = ManuallyDrop::new(self);
            Ok(None)
        }
    }
}

impl<R: Send + Sync, Store: SessionStore<R>> Deref for DataMut<R, Store> {
    type Target = R;

    fn deref(&self) -> &Self::Target {
        &self.session.data
    }
}

impl<R: Send + Sync, Store: SessionStore<R>> DerefMut for DataMut<R, Store> {
    fn deref_mut(&mut self) -> &mut R {
        &mut self.session.data
    }
}

impl<R, Store> Drop for DataMut<R, Store>
where
    R: Send + Sync,
    Store: SessionStore<R>,
{
    fn drop(&mut self) {
        let _ = self
            .session
            .store
            .save(&self.session.id, &self.session.data);
    }
}

/// ID type for sessions.
///
/// Wraps an array of 16 bytes.
///
/// # Examples
///
/// ```rust
/// use tower_sessions::session::Id;
///
/// Id::default();
/// ```
#[cfg(feature = "id-access")]
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
pub struct Id(pub i128);

/// ID type for sessions.
///
/// Wraps an array of 16 bytes.
///
/// # Examples
///
/// ```rust
/// use tower_sessions::session::Id;
///
/// Id::default();
/// ```
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
#[cfg(not(feature = "id-access"))]
pub struct Id(i128);

impl Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut encoded = [0; 22];
        URL_SAFE_NO_PAD
            .encode_slice(self.0.to_le_bytes(), &mut encoded)
            .expect("Encoded ID must be exactly 22 bytes");
        let encoded = str::from_utf8(&encoded).expect("Encoded ID must be valid UTF-8");

        f.write_str(encoded)
    }
}

enum SessionUpdate {
    Delete,
    Set(Id),
}

type Updater = Arc<Mutex<Option<SessionUpdate>>>;
