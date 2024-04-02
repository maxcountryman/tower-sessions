//! A session which allows HTTP applications to associate data with visitors.
use std::{
    collections::HashMap,
    fmt::{self, Display},
    hash::Hash,
    result,
    str::{self, FromStr},
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, DecodeError, Engine as _};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use time::{Duration, OffsetDateTime};
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};

use crate::{session_store, SessionStore};

const DEFAULT_DURATION: Duration = Duration::weeks(2);

type Result<T> = result::Result<T, Error>;

type Data = HashMap<String, Value>;

/// Session errors.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Maps `serde_json` errors.
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    /// Maps `session_store::Error` errors.
    #[error(transparent)]
    Store(#[from] session_store::Error),
}

#[derive(Debug)]
struct Inner {
    // This will be `None` when:
    //
    // 1. We have not been provided a session cookie or have failed to parse it,
    // 2. The store has not found the session.
    //
    // Sync lock, see: https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html#which-kind-of-mutex-should-you-use
    session_id: parking_lot::Mutex<Option<Id>>,

    // A lazy representation of the session's value, hydrated on a just-in-time basis. A
    // `None` value indicates we have not tried to access it yet. After access, it will always
    // contain `Some(Record)`.
    record: Mutex<Option<Record>>,

    // Sync lock, see: https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html#which-kind-of-mutex-should-you-use
    expiry: parking_lot::Mutex<Option<Expiry>>,

    is_modified: AtomicBool,
}

/// A session which allows HTTP applications to associate key-value pairs with
/// visitors.
#[derive(Debug, Clone)]
pub struct Session {
    store: Arc<dyn SessionStore>,
    inner: Arc<Inner>,
}

impl Session {
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
    pub fn new(
        session_id: Option<Id>,
        store: Arc<impl SessionStore>,
        expiry: Option<Expiry>,
    ) -> Self {
        let inner = Inner {
            session_id: parking_lot::Mutex::new(session_id),
            record: Mutex::new(None), // `None` indicates we have not loaded from store.
            expiry: parking_lot::Mutex::new(expiry),
            is_modified: AtomicBool::new(false),
        };

        Self {
            store,
            inner: Arc::new(inner),
        }
    }

    fn create_record(&self) -> Record {
        Record::new(self.expiry_date())
    }

    #[tracing::instrument(skip(self), err)]
    async fn get_record(&self) -> Result<MappedMutexGuard<Record>> {
        let mut record_guard = self.inner.record.lock().await;

        // Lazily load the record since `None` here indicates we have no yet loaded it.
        if record_guard.is_none() {
            tracing::trace!("record not loaded from store; loading");

            let session_id = *self.inner.session_id.lock();
            *record_guard = Some(if let Some(session_id) = session_id {
                match self.store.load(&session_id).await? {
                    Some(loaded_record) => {
                        tracing::trace!("record found in store");
                        loaded_record
                    }

                    None => {
                        // A well-behaved user agent should not send session cookies after
                        // expiration. Even so it's possible for an expired session to be removed
                        // from the store after a request was initiated. However, such a race should
                        // be relatively uncommon and as such entering this branch could indicate
                        // malicious behavior.
                        tracing::warn!("possibly suspicious activity: record not found in store");
                        *self.inner.session_id.lock() = None;
                        self.create_record()
                    }
                }
            } else {
                tracing::trace!("session id not found");
                self.create_record()
            })
        }

        Ok(MutexGuard::map(record_guard, |opt| {
            opt.as_mut()
                .expect("Record should always be `Option::Some` at this point")
        }))
    }

    /// Inserts a `impl Serialize` value into the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value = session.get::<usize>("foo").await.unwrap();
    /// assert_eq!(value, Some(42));
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - This method can fail when [`serde_json::to_value`] fails.
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn insert(&self, key: &str, value: impl Serialize) -> Result<()> {
        self.insert_value(key, serde_json::to_value(&value)?)
            .await?;
        Ok(())
    }

    /// Inserts a `serde_json::Value` into the session.
    ///
    /// If the key was not present in the underlying map, `None` is returned and
    /// `modified` is set to `true`.
    ///
    /// If the underlying map did have the key and its value is the same as the
    /// provided value, `None` is returned and `modified` is not set.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// let value = session
    ///     .insert_value("foo", serde_json::json!(42))
    ///     .await
    ///     .unwrap();
    /// assert!(value.is_none());
    ///
    /// let value = session
    ///     .insert_value("foo", serde_json::json!(42))
    ///     .await
    ///     .unwrap();
    /// assert!(value.is_none());
    ///
    /// let value = session
    ///     .insert_value("foo", serde_json::json!("bar"))
    ///     .await
    ///     .unwrap();
    /// assert_eq!(value, Some(serde_json::json!(42)));
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn insert_value(&self, key: &str, value: Value) -> Result<Option<Value>> {
        let mut record_guard = self.get_record().await?;
        Ok(if record_guard.data.get(key) != Some(&value) {
            self.inner
                .is_modified
                .store(true, atomic::Ordering::Release);
            record_guard.data.insert(key.to_string(), value)
        } else {
            None
        })
    }

    /// Gets a value from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value = session.get::<usize>("foo").await.unwrap();
    /// assert_eq!(value, Some(42));
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - This method can fail when [`serde_json::from_value`] fails.
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self
            .get_value(key)
            .await?
            .map(serde_json::from_value)
            .transpose()?)
    }

    /// Gets a `serde_json::Value` from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value = session.get_value("foo").await.unwrap().unwrap();
    /// assert_eq!(value, serde_json::json!(42));
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn get_value(&self, key: &str) -> Result<Option<Value>> {
        let record_guard = self.get_record().await?;
        Ok(record_guard.data.get(key).cloned())
    }

    /// Removes a value from the store, retuning the value of the key if it was
    /// present in the underlying map.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value: Option<usize> = session.remove("foo").await.unwrap();
    /// assert_eq!(value, Some(42));
    ///
    /// let value: Option<usize> = session.get("foo").await.unwrap();
    /// assert!(value.is_none());
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - This method can fail when [`serde_json::from_value`] fails.
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn remove<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self
            .remove_value(key)
            .await?
            .map(serde_json::from_value)
            .transpose()?)
    }

    /// Removes a `serde_json::Value` from the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    /// let value = session.remove_value("foo").await.unwrap().unwrap();
    /// assert_eq!(value, serde_json::json!(42));
    ///
    /// let value: Option<usize> = session.get("foo").await.unwrap();
    /// assert!(value.is_none());
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn remove_value(&self, key: &str) -> Result<Option<Value>> {
        let mut record_guard = self.get_record().await?;
        self.inner
            .is_modified
            .store(true, atomic::Ordering::Release);
        Ok(record_guard.data.remove(key))
    }

    /// Clears the session of all data but does not delete it from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    ///
    /// let session = Session::new(None, store.clone(), None);
    /// session.insert("foo", 42).await.unwrap();
    /// assert!(!session.is_empty().await);
    ///
    /// session.save().await.unwrap();
    ///
    /// session.clear().await;
    ///
    /// // Not empty! (We have an ID still.)
    /// assert!(!session.is_empty().await);
    /// // Data is cleared...
    /// assert!(session.get::<usize>("foo").await.unwrap().is_none());
    ///
    /// // ...data is cleared before loading from the backend...
    /// let session = Session::new(session.id(), store.clone(), None);
    /// session.clear().await;
    /// assert!(session.get::<usize>("foo").await.unwrap().is_none());
    ///
    /// let session = Session::new(session.id(), store, None);
    /// // ...but data is not deleted from the store.
    /// assert_eq!(session.get::<usize>("foo").await.unwrap(), Some(42));
    /// # });
    /// ```
    pub async fn clear(&self) {
        let mut record_guard = self.inner.record.lock().await;
        if let Some(record) = record_guard.as_mut() {
            record.data.clear();
        } else if let Some(session_id) = *self.inner.session_id.lock() {
            let mut new_record = self.create_record();
            new_record.id = session_id;
            *record_guard = Some(new_record);
        }

        self.inner
            .is_modified
            .store(true, atomic::Ordering::Release);
    }

    /// Returns `true` if there is no session ID and the session is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{session::Id, MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    ///
    /// let session = Session::new(None, store.clone(), None);
    /// // Empty if we have no ID and record is not loaded.
    /// assert!(session.is_empty().await);
    ///
    /// let session = Session::new(Some(Id::default()), store.clone(), None);
    /// // Not empty if we have an ID but no record. (Record is not loaded here.)
    /// assert!(!session.is_empty().await);
    ///
    /// let session = Session::new(Some(Id::default()), store.clone(), None);
    /// session.insert("foo", 42).await.unwrap();
    /// // Not empty after inserting.
    /// assert!(!session.is_empty().await);
    /// session.save().await.unwrap();
    /// // Not empty after saving.
    /// assert!(!session.is_empty().await);
    ///
    /// let session = Session::new(session.id(), store.clone(), None);
    /// session.load().await.unwrap();
    /// // Not empty after loading from store...
    /// assert!(!session.is_empty().await);
    /// // ...and not empty after accessing the session.
    /// session.get::<usize>("foo").await.unwrap();
    /// assert!(!session.is_empty().await);
    ///
    /// let session = Session::new(session.id(), store.clone(), None);
    /// session.delete().await.unwrap();
    /// // Not empty after deleting from store...
    /// assert!(!session.is_empty().await);
    /// session.get::<usize>("foo").await.unwrap();
    /// // ...but empty after trying to access the deleted session.
    /// assert!(session.is_empty().await);
    ///
    /// let session = Session::new(None, store, None);
    /// session.insert("foo", 42).await.unwrap();
    /// session.flush().await.unwrap();
    /// // Empty after flushing.
    /// assert!(session.is_empty().await);
    /// # });
    /// ```
    pub async fn is_empty(&self) -> bool {
        let record_guard = self.inner.record.lock().await;

        // N.B.: Session IDs are `None` if:
        //
        // 1. The cookie was not provided or otherwise could not be parsed,
        // 2. Or the session could not be loaded from the store.
        let session_id = self.inner.session_id.lock();

        let Some(record) = record_guard.as_ref() else {
            return session_id.is_none();
        };

        session_id.is_none() && record.data.is_empty()
    }

    /// Get the session ID.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{session::Id, MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    ///
    /// let session = Session::new(None, store.clone(), None);
    /// assert!(session.id().is_none());
    ///
    /// let id = Some(Id::default());
    /// let session = Session::new(id, store, None);
    /// assert_eq!(id, session.id());
    /// ```
    pub fn id(&self) -> Option<Id> {
        *self.inner.session_id.lock()
    }

    /// Get the session expiry.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{session::Expiry, MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// assert_eq!(session.expiry(), None);
    /// ```
    pub fn expiry(&self) -> Option<Expiry> {
        *self.inner.expiry.lock()
    }

    /// Set `expiry` to the given value.
    ///
    /// This may be used within applications directly to alter the session's
    /// time to live.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use time::OffsetDateTime;
    /// use tower_sessions::{session::Expiry, MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// let expiry = Expiry::AtDateTime(OffsetDateTime::now_utc());
    /// session.set_expiry(Some(expiry));
    ///
    /// assert_eq!(session.expiry(), Some(expiry));
    /// ```
    pub fn set_expiry(&self, expiry: Option<Expiry>) {
        *self.inner.expiry.lock() = expiry;
        self.inner
            .is_modified
            .store(true, atomic::Ordering::Release);
    }

    /// Get session expiry as `OffsetDateTime`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use time::{Duration, OffsetDateTime};
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// // Our default duration is two weeks.
    /// let expected_expiry = OffsetDateTime::now_utc().saturating_add(Duration::weeks(2));
    ///
    /// assert!(session.expiry_date() > expected_expiry.saturating_sub(Duration::seconds(1)));
    /// assert!(session.expiry_date() < expected_expiry.saturating_add(Duration::seconds(1)));
    /// ```
    pub fn expiry_date(&self) -> OffsetDateTime {
        let expiry = self.inner.expiry.lock();
        match *expiry {
            Some(Expiry::OnInactivity(duration)) => {
                OffsetDateTime::now_utc().saturating_add(duration)
            }
            Some(Expiry::AtDateTime(datetime)) => datetime,
            Some(Expiry::OnSessionEnd) | None => {
                OffsetDateTime::now_utc().saturating_add(DEFAULT_DURATION) // TODO: The default should probably be configurable.
            }
        }
    }

    /// Get session expiry as `Duration`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use time::Duration;
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// let expected_duration = Duration::weeks(2);
    ///
    /// assert!(session.expiry_age() > expected_duration.saturating_sub(Duration::seconds(1)));
    /// assert!(session.expiry_age() < expected_duration.saturating_add(Duration::seconds(1)));
    /// ```
    pub fn expiry_age(&self) -> Duration {
        std::cmp::max(
            self.expiry_date() - OffsetDateTime::now_utc(),
            Duration::ZERO,
        )
    }

    /// Returns `true` if the session has been modified during the request.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store, None);
    ///
    /// // Not modified initially.
    /// assert!(!session.is_modified());
    ///
    /// // Getting doesn't count as a modification.
    /// session.get::<usize>("foo").await.unwrap();
    /// assert!(!session.is_modified());
    ///
    /// // Insertions and removals do though.
    /// session.insert("foo", 42).await.unwrap();
    /// assert!(session.is_modified());
    /// # });
    /// ```
    pub fn is_modified(&self) -> bool {
        self.inner.is_modified.load(atomic::Ordering::Acquire)
    }

    /// Saves the session record to the store.
    ///
    /// Note that this method is generally not needed and is reserved for
    /// situations where the session store must be updated during the
    /// request.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    /// session.save().await.unwrap();
    ///
    /// let session = Session::new(session.id(), store, None);
    /// assert_eq!(session.get::<usize>("foo").await.unwrap().unwrap(), 42);
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If saving to the store fails, we fail with [`Error::Store`].
    #[tracing::instrument(skip(self), err)]
    pub async fn save(&self) -> Result<()> {
        let mut record_guard = self.get_record().await?;
        record_guard.expiry_date = self.expiry_date();

        // Session ID is `None` if:
        //
        //  1. No valid cookie was found on the request or,
        //  2. No valid session was found in the store.
        //
        // In either case, we must create a new session via the store interface.
        //
        // Potential ID collisions must be handled by session store implementers.
        if self.inner.session_id.lock().is_none() {
            self.store.create(&mut record_guard).await?;
            *self.inner.session_id.lock() = Some(record_guard.id);
        } else {
            self.store.save(&record_guard).await?;
        }
        Ok(())
    }

    /// Loads the session record from the store.
    ///
    /// Note that this method is generally not needed and is reserved for
    /// situations where the session must be updated during the request.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{session::Id, MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let id = Some(Id::default());
    /// let session = Session::new(id, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    /// session.save().await.unwrap();
    ///
    /// let session = Session::new(session.id(), store, None);
    /// session.load().await.unwrap();
    ///
    /// assert_eq!(session.get::<usize>("foo").await.unwrap().unwrap(), 42);
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If loading from the store fails, we fail with [`Error::Store`].
    #[tracing::instrument(skip(self), err)]
    pub async fn load(&self) -> Result<()> {
        let session_id = *self.inner.session_id.lock();
        let Some(ref id) = session_id else {
            tracing::warn!("called load with no session id");
            return Ok(());
        };
        let loaded_record = self.store.load(id).await.map_err(Error::Store)?;
        let mut record_guard = self.inner.record.lock().await;
        *record_guard = loaded_record;
        Ok(())
    }

    /// Deletes the session from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{session::Id, MemoryStore, Session, SessionStore};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(Some(Id::default()), store.clone(), None);
    ///
    /// // Save before deleting.
    /// session.save().await.unwrap();
    ///
    /// // Delete from the store.
    /// session.delete().await.unwrap();
    ///
    /// assert!(store.load(&session.id().unwrap()).await.unwrap().is_none());
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If deleting from the store fails, we fail with [`Error::Store`].
    #[tracing::instrument(skip(self), err)]
    pub async fn delete(&self) -> Result<()> {
        let session_id = *self.inner.session_id.lock();
        let Some(ref session_id) = session_id else {
            tracing::warn!("called delete with no session id");
            return Ok(());
        };
        self.store.delete(session_id).await.map_err(Error::Store)?;
        Ok(())
    }

    /// Flushes the session by removing all data contained in the session and
    /// then deleting it from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{MemoryStore, Session, SessionStore};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", "bar").await.unwrap();
    /// session.save().await.unwrap();
    ///
    /// let id = session.id().unwrap();
    ///
    /// session.flush().await.unwrap();
    ///
    /// assert!(session.id().is_none());
    /// assert!(session.is_empty().await);
    /// assert!(store.load(&id).await.unwrap().is_none());
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If deleting from the store fails, we fail with [`Error::Store`].
    pub async fn flush(&self) -> Result<()> {
        self.clear().await;
        self.delete().await?;
        *self.inner.session_id.lock() = None;
        Ok(())
    }

    /// Cycles the session ID while retaining any data that was associated with
    /// it.
    ///
    /// Using this method helps prevent session fixation attacks by ensuring a
    /// new ID is assigned to the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// use std::sync::Arc;
    ///
    /// use tower_sessions::{session::Id, MemoryStore, Session};
    ///
    /// let store = Arc::new(MemoryStore::default());
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    /// session.save().await.unwrap();
    /// let id = session.id();
    ///
    /// let session = Session::new(session.id(), store.clone(), None);
    /// session.cycle_id().await.unwrap();
    ///
    /// assert!(!session.is_empty().await);
    /// assert!(session.is_modified());
    ///
    /// session.save().await.unwrap();
    ///
    /// let session = Session::new(session.id(), store, None);
    ///
    /// assert_ne!(id, session.id());
    /// assert_eq!(session.get::<usize>("foo").await.unwrap().unwrap(), 42);
    /// # });
    /// ```
    ///
    /// # Errors
    ///
    /// - If deleting from the store fails or saving to the store fails, we fail
    ///   with [`Error::Store`].
    pub async fn cycle_id(&self) -> Result<()> {
        let mut record_guard = self.get_record().await?;

        let old_session_id = record_guard.id;
        record_guard.id = Id::default();
        *self.inner.session_id.lock() = None; // Setting `None` ensures `save` invokes the store's
                                              // `create` method.

        self.store
            .delete(&old_session_id)
            .await
            .map_err(Error::Store)?;

        self.inner
            .is_modified
            .store(true, atomic::Ordering::Release);

        Ok(())
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
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
pub struct Id(pub i128); // TODO: By this being public, it may be possible to override the
                         // session ID, which is undesirable.

impl Default for Id {
    fn default() -> Self {
        use rand::prelude::*;

        Self(rand::thread_rng().gen())
    }
}

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

impl FromStr for Id {
    type Err = base64::DecodeSliceError;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        let mut decoded = [0; 16];
        let bytes_decoded = URL_SAFE_NO_PAD.decode_slice(s.as_bytes(), &mut decoded)?;
        if bytes_decoded != 16 {
            let err = DecodeError::InvalidLength(bytes_decoded);
            return Err(base64::DecodeSliceError::DecodeError(err));
        }

        Ok(Self(i128::from_le_bytes(decoded)))
    }
}

/// Record type that's appropriate for encoding and decoding sessions to and
/// from session stores.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub id: Id,
    pub data: Data,
    pub expiry_date: OffsetDateTime,
}

impl Record {
    fn new(expiry_date: OffsetDateTime) -> Self {
        Self {
            id: Id::default(),
            data: Data::default(),
            expiry_date,
        }
    }
}

/// Session expiry configuration.
///
/// # Examples
///
/// ```rust
/// use time::{Duration, OffsetDateTime};
/// use tower_sessions::Expiry;
///
/// // Will be expired on "session end".
/// let expiry = Expiry::OnSessionEnd;
///
/// // Will be expired in five minutes from last acitve.
/// let expiry = Expiry::OnInactivity(Duration::minutes(5));
///
/// // Will be expired at the given timestamp.
/// let expired_at = OffsetDateTime::now_utc().saturating_add(Duration::weeks(2));
/// let expiry = Expiry::AtDateTime(expired_at);
/// ```
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Expiry {
    /// Expire on [current session end][current-session-end], as defined by the
    /// browser.
    ///
    /// [current-session-end]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Cookies#define_the_lifetime_of_a_cookie
    OnSessionEnd,

    /// Expire on inactivity.
    ///
    /// Reading a session is not considered activity for expiration purposes.
    /// [`Session`] expiration is computed from the last time the session was
    /// _modified_.
    OnInactivity(Duration),

    /// Expire at a specific date and time.
    ///
    /// This value may be extended manually with
    /// [`set_expiry`](Session::set_expiry).
    AtDateTime(OffsetDateTime),
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use mockall::{
        mock,
        predicate::{self, always},
    };

    use super::*;

    mock! {
        #[derive(Debug)]
        pub Store {}

        #[async_trait]
        impl SessionStore for Store {
            async fn create(&self, record: &mut Record) -> session_store::Result<()>;
            async fn save(&self, record: &Record) -> session_store::Result<()>;
            async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>>;
            async fn delete(&self, session_id: &Id) -> session_store::Result<()>;
        }
    }

    #[tokio::test]
    async fn test_cycle_id() {
        let mut mock_store = MockStore::new();

        let initial_id = Id::default();
        let new_id = Id::default();

        // Set up expectations for the mock store
        mock_store
            .expect_save()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));
        mock_store
            .expect_load()
            .with(predicate::eq(initial_id))
            .times(1)
            .returning(move |_| {
                Ok(Some(Record {
                    id: initial_id,
                    data: Data::default(),
                    expiry_date: OffsetDateTime::now_utc(),
                }))
            });
        mock_store
            .expect_delete()
            .with(predicate::eq(initial_id))
            .times(1)
            .returning(|_| Ok(()));
        mock_store
            .expect_create()
            .times(1)
            .returning(move |record| {
                record.id = new_id;
                Ok(())
            });

        let store = Arc::new(mock_store);
        let session = Session::new(Some(initial_id), store.clone(), None);

        // Insert some data and save the session
        session.insert("foo", 42).await.unwrap();
        session.save().await.unwrap();

        // Cycle the session ID
        session.cycle_id().await.unwrap();

        // Verify that the session ID has changed and the data is still present
        assert_ne!(session.id(), Some(initial_id));
        assert!(session.id().is_none()); // The session ID should be None
        assert_eq!(session.get::<i32>("foo").await.unwrap(), Some(42));

        // Save the session to update the ID in the session object
        session.save().await.unwrap();
        assert_eq!(session.id(), Some(new_id));
    }
}
