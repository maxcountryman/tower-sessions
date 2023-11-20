//! A session which allows HTTP applications to associate data with visitors.
use std::{
    collections::HashMap,
    fmt::Display,
    hash::Hash,
    str::FromStr,
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use time::Duration;
use tokio::sync::{Mutex, MutexGuard};
use tower_cookies::cookie::time::OffsetDateTime;
use uuid::Uuid;

use crate::SessionStore;

const DEFAULT_DURATION: Duration = Duration::weeks(2);

type Result<T, Store> = std::result::Result<T, Error<Store>>;

type Data = HashMap<String, Value>;

/// Session errors.
#[derive(thiserror::Error, Debug)]
pub enum Error<Store: SessionStore> {
    /// Maps `serde_json` errors.
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    /// Maps `SessionStore::Error` errors.
    #[error(transparent)]
    Store(Store::Error),
}

/// A session which allows HTTP applications to associate key-value pairs with
/// visitors.
#[derive(Debug, Clone)]
pub struct Session<Store: SessionStore> {
    session_id: Id,
    store: Store,
    record: Arc<Mutex<Option<Record>>>,

    // See: https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html#which-kind-of-mutex-should-you-use
    expiry: Arc<parking_lot::Mutex<Option<Expiry>>>,

    is_modified: Arc<AtomicBool>,
}

impl<Store: SessionStore> Session<Store> {
    /// Creates a new session with the cookie ID, store, and expiry.
    ///
    /// This method is lazy and does not invoke the overhead of talking to the
    /// backing store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// Session::new(None, store.clone(), None);
    /// ```
    pub fn new(session_id: Id, store: Store, expiry: Option<Expiry>) -> Self {
        Self {
            session_id,
            store,
            record: Arc::new(Mutex::new(None)), // `None` indicates we have not loaded from store.
            expiry: Arc::new(parking_lot::Mutex::new(expiry)),
            is_modified: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn create_record(&self) -> Record {
        Record::new(self.expiry_date())
    }

    #[tracing::instrument(skip(self), err)]
    async fn record(&self) -> Result<MutexGuard<Option<Record>>, Store> {
        let mut record_guard = self.record.lock().await;

        // Lazily load the record from the store.
        if record_guard.is_none() {
            tracing::trace!("record not loaded from store; loading");

            *record_guard = match self
                .store
                .load(&self.session_id)
                .await
                .map_err(Error::Store)?
            {
                Some(loaded_record) => {
                    tracing::trace!("record found in store");
                    Some(loaded_record)
                }
                None => {
                    tracing::trace!("record not found in store");
                    let new_record = self.create_record().await;
                    Some(new_record)
                }
            }
        }

        Ok(record_guard)
    }

    /// Inserts a `impl Serialize` value into the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value = session.get::<usize>("foo").await.unwrap();
    /// assert_eq!(value, Some(42));
    /// ```
    ///
    /// # Errors
    ///
    /// - This method can fail when [`serde_json::to_value`] fails.
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn insert(&self, key: &str, value: impl Serialize) -> Result<(), Store> {
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
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
    /// ```
    ///
    /// # Errors
    ///
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn insert_value(&self, key: &str, value: Value) -> Result<Option<Value>, Store> {
        Ok(self.record().await?.as_mut().and_then(|record| {
            if record.data.get(key) != Some(&value) {
                self.is_modified.store(true, atomic::Ordering::Release);
                record.data.insert(key.to_string(), value)
            } else {
                None
            }
        }))
    }

    /// Gets a value from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value = session.get::<usize>("foo").await.unwrap();
    /// assert_eq!(value, Some(42));
    /// ```
    ///
    /// # Errors
    ///
    /// - This method can fail when [`serde_json::from_value`] fails.
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, Store> {
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value = session.get_value("foo").await.unwrap();
    /// assert_eq!(value, serde_json::json!(42));
    /// ```
    ///
    /// # Errors
    ///
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn get_value(&self, key: &str) -> Result<Option<Value>, Store> {
        Ok(self
            .record()
            .await?
            .as_ref()
            .and_then(|record| record.data.get(key).cloned()))
    }

    /// Removes a value from the store, retuning the value of the key if it was
    /// present in the underlying map.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    ///
    /// let value: Option<usize> = session.remove("foo").await.unwrap();
    /// assert_eq!(value, Some(42));
    ///
    /// let value: Option<usize> = session.get("foo").await.unwrap();
    /// assert!(value.is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// - This method can fail when [`serde_json::from_value`] fails.
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn remove<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, Store> {
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// let value = session.remove_value("foo").await.unwrap();
    /// assert_eq!(value, serde_json::json!(42));
    ///
    /// let value: Option<usize> = session.get("foo").await.unwrap();
    /// assert!(value.is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// - If the session has not been hydrated and loading from the store fails,
    ///   we fail with [`Error::Store`].
    pub async fn remove_value(&self, key: &str) -> Result<Option<Value>, Store> {
        Ok(self.record().await?.as_mut().and_then(|record| {
            self.is_modified.store(true, atomic::Ordering::Release);
            record.data.remove(key)
        }))
    }

    /// Clears the session of all data but does not delete it from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// session.insert("foo", 42).await.unwrap();
    /// session.clear().await;
    ///
    /// assert!(session.is_empty().await);
    /// ```
    pub async fn clear(&self) {
        let mut record = self.record.lock().await;
        if let Some(record) = record.as_mut() {
            record.data.clear();
        }
        self.is_modified.store(true, atomic::Ordering::Release);
    }

    /// Returns `true` if there is no cookie ID and the session is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// assert!(session.is_empty().await, true);
    ///
    /// session.insert("foo", "bar").await.unwrap();
    ///
    /// assert!(session.is_empty().await, false);
    /// ```
    pub async fn is_empty(&self) -> bool {
        // N.B.: We do not load from the store here, so if we haven't loaded at all, we
        // assume that the presence of cookie ID indicates there may be a
        // session and therefore will not return `true` from this method.
        let record = self.record.lock().await;
        record.as_ref().is_some_and(|record| record.data.is_empty())
    }

    /// Get the session ID.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(store.clone(), Id::default(), None);
    ///
    /// assert!(session.id().await.is_none());
    /// ```
    pub async fn id(&self) -> Id {
        self.record
            .lock()
            .await
            .as_ref()
            .map(|record| record.id)
            .unwrap_or(self.session_id)
    }

    /// Get the session expiry.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::Expiry, MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// assert_eq!(session.expiry(), None);
    /// ```
    pub fn expiry(&self) -> Option<Expiry> {
        *self.expiry.lock()
    }

    /// Set `expiry` give the given value.
    ///
    /// This may be used within applications directly to alter the session's
    /// time to live.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::OffsetDateTime;
    /// use tower_sessions::{session::Expiry, MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// let expiry = Expiry::AtDateTime(OffsetDateTime::now_utc());
    /// session.set_expiry(expiry);
    ///
    /// assert_eq!(session.expiry(), expiry);
    /// ```
    pub fn set_expiry(&self, expiry: Option<Expiry>) {
        let mut current_expiry = self.expiry.lock();
        *current_expiry = expiry;
    }

    /// Get session expiry as `OffsetDateTime`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// assert_eq!(session.expiry_date(), DEFAULT_DURATION);
    /// ```
    pub fn expiry_date(&self) -> OffsetDateTime {
        let expiry = self.expiry.lock();
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// assert_eq!(session.expiry_age(), DEFAULT_DURATION);
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// // Not modified initially.
    /// assert!(!session.is_modified().await.unwrap());
    ///
    /// // Getting doesn't count as a modification.
    /// session.get("foo").await.unwrap();
    /// assert!(!session.is_modified().await.unwrap());
    ///
    /// // Insertions and removals do though.
    /// session.insert("foo", "bar").await.unwrap();
    /// assert!(session.is_modified().await.unwrap());
    /// ```
    pub fn is_modified(&self) -> bool {
        self.is_modified.load(atomic::Ordering::Acquire)
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    /// session.insert("foo", "bar").await.unwrap();
    /// session.save().await.unwrap();
    ///
    /// let session = Session::new(None, store.clone(), None);
    ///
    /// assert_eq!(session.get("foo").await.unwrap(), Some("bar"));
    /// ```
    ///
    /// # Errors
    ///
    /// - If loading from or saving to the store fails, we fail with
    ///   [`Error::Store`].
    pub async fn save(&self) -> Result<(), Store> {
        if let Some(record) = self.record().await?.as_ref() {
            self.store.save(record).await.map_err(Error::Store)?;
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store.clone(), None);
    /// session.insert("foo", "bar").await.unwrap();
    /// session.save().await.unwrap();
    ///
    /// let session = Session::new(None, store.clone(), None);
    /// session.load().await.unwrap();
    ///
    /// assert_eq!(session.get("foo").await.unwrap(), Some("bar"));
    /// ```
    ///
    /// # Errors
    ///
    /// - If loading from the store fails, we fail with [`Error::Store`].
    pub async fn load(&self) -> Result<(), Store> {
        let loaded_record = self
            .store
            .load(&self.id().await)
            .await
            .map_err(Error::Store)?;
        let mut record = self.record().await?;
        *record = loaded_record;
        Ok(())
    }

    /// Deletes the session from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store, None);
    ///
    /// session.delete().await.unwrap();
    ///
    /// assert!(store.load(session.id().await.unwrap()).unwrap().is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// - If deleting from the store fails, we fail with [`Error::Store`].
    pub async fn delete(&self) -> Result<(), Store> {
        self.store
            .delete(&self.id().await)
            .await
            .map_err(Error::Store)?;
        Ok(())
    }

    /// Flushes the session by removing all data contained in the session and
    /// then deleting it from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store, None);
    ///
    /// session.insert("foo", "bar").await.unwrap();
    /// session.flush().await.unwrap();
    ///
    /// assert!(session.is_empty().await.unwrap());
    /// assert!(store.load(session.id().await.unwrap()).unwrap().is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// - If deleting from the store fails, we fail with [`Error::Store`].
    pub async fn flush(&self) -> Result<(), Store> {
        self.clear().await;
        self.delete().await?;
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
    /// use tower_sessions::{MemoryStore, Session};
    ///
    /// let store = MemoryStore::default();
    /// let session = Session::new(None, store, None);
    ///
    /// session.insert("foo", "bar").await.unwrap();
    ///
    /// let id = session.id().await.unwrap();
    ///
    /// session.cycle_id().await.unwrap();
    ///
    /// assert_ne!(Some(id), session.id().await);
    /// assert_eq!(session.get("foo").await.unwrap(), Some("foo"));
    /// ```
    ///
    /// # Errors
    ///
    /// - If deleting from the store fails or saving to the store fails, we fail
    ///   with [`Error::Store`].
    pub async fn cycle_id(&self) -> Result<(), Store> {
        self.delete().await?;

        {
            let mut record_guard = self.record.lock().await;
            match record_guard.as_mut() {
                Some(record) => {
                    record.id = Id::default();
                }
                None => {
                    let record = self.create_record().await;
                    *record_guard = Some(record);
                }
            }
        }

        self.save().await?;
        self.is_modified.store(true, atomic::Ordering::Release);

        Ok(())
    }
}

/// ID type for sessions.
///
/// Wraps a UUIDv4.
///
/// # Examples
///
/// ```rust
/// use tower_sessions::session::Id;
///
/// Id::default();
/// ```
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
pub struct Id(pub Uuid); // TODO: By this being public, it may be possible to override UUIDv4,
                         // which is undesirable.

impl Default for Id {
    fn default() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.as_hyphenated().to_string())
    }
}

impl FromStr for Id {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.parse::<uuid::Uuid>()?))
    }
}

/// Record type that's appropriate for encoding and decoding sessions to and
/// from session stores.
///
/// # Examples
///
/// ```rust
/// use time::OffsetDateTime;
/// use tower_sessions::session::{Data, Id, Record};
///
/// Record {
///     id: Id::default(),
///     data: Data::default(),
///     expiry_date: OffsetDateTime::now_utc(),
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
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
