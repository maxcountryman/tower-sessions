//! A session which allows HTTP applications to associate data with visitors.
use std::{collections::HashMap, fmt::Display, sync::Arc};

use parking_lot::Mutex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use time::Duration;
use tower_cookies::cookie::time::OffsetDateTime;
use uuid::Uuid;

use crate::CookieConfig;

/// Session errors.
#[derive(thiserror::Error, Debug)]
pub enum SessionError {
    /// A variant to map `uuid` errors.
    #[error("Invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    /// A variant to map `serde_json` errors.
    #[error("JSON serialization/deserialization error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
}

type SessionResult<T> = Result<T, SessionError>;

/// A session which allows HTTP applications to associate key-value pairs with
/// visitors.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub(crate) id: SessionId,
    expiration_time: Option<OffsetDateTime>,
    inner: Arc<Mutex<Inner>>,
}

impl Session {
    /// Create a new session with defaults.
    ///
    /// Note that an `expiration_time` of none results in a cookie with
    /// expiration `"Session"`.
    ///
    /// # Examples
    ///
    ///```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// ```
    pub fn new() -> Self {
        let inner = Inner {
            data: HashMap::new(),
            modified: false,
            deleted: None,
        };

        Self {
            id: SessionId::default(),
            expiration_time: None,
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// A method for setting `expiration_time` in accordance with `max_age`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::Duration;
    /// use tower_sessions::Session;
    /// let session = Session::new().with_max_age(Duration::minutes(5));
    /// ```
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        let expiration_time = OffsetDateTime::now_utc().saturating_add(max_age);
        self.expiration_time = Some(expiration_time);
        self
    }

    /// Inserts a `impl Serialize` value into the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.insert("foo", 42).expect("Serialization error.");
    /// ```
    ///
    /// # Errors
    ///
    /// This method can fail when [`serde_json::to_value`] fails.
    pub fn insert(&self, key: &str, value: impl Serialize) -> SessionResult<()> {
        self.insert_value(key, serde_json::to_value(&value)?);
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
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// let value = session.insert_value("foo", serde_json::json!(42));
    /// assert!(value.is_none());
    ///
    /// let value = session.insert_value("foo", serde_json::json!(42));
    /// assert!(value.is_none());
    ///
    /// let value = session.insert_value("foo", serde_json::json!("bar"));
    /// assert_eq!(value, Some(serde_json::json!(42)));
    /// ```
    pub fn insert_value(&self, key: &str, value: Value) -> Option<Value> {
        let mut inner = self.inner.lock();
        if inner.data.get(key) != Some(&value) {
            inner.modified = true;
            inner.data.insert(key.to_string(), value)
        } else {
            None
        }
    }

    /// Gets a value from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.insert("foo", 42).unwrap();
    /// let value = session.get::<usize>("foo").unwrap();
    /// assert_eq!(value, Some(42));
    /// ```
    ///
    /// # Errors
    ///
    /// This method can fail when [`serde_json::from_value`] fails.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> SessionResult<Option<T>> {
        Ok(self
            .get_value(key)
            .map(serde_json::from_value)
            .transpose()?)
    }

    /// Gets a `serde_json::Value` from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.insert("foo", 42).unwrap();
    /// let value = session.get_value("foo").unwrap();
    /// assert_eq!(value, serde_json::json!(42));
    /// ```
    pub fn get_value(&self, key: &str) -> Option<Value> {
        let inner = self.inner.lock();
        inner.data.get(key).cloned()
    }

    /// Removes a value from the store, retuning the value of the key if it was
    /// present in the underlying map.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.insert("foo", 42).unwrap();
    /// let value: Option<usize> = session.remove("foo").unwrap();
    /// assert_eq!(value, Some(42));
    /// let value: Option<usize> = session.get("foo").unwrap();
    /// assert!(value.is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// This method can fail when [`serde_json::from_value`] fails.
    pub fn remove<T: DeserializeOwned>(&self, key: &str) -> SessionResult<Option<T>> {
        Ok(self
            .remove_value(key)
            .map(serde_json::from_value)
            .transpose()?)
    }

    /// Removes a `serde_json::Value` from the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.insert("foo", 42).unwrap();
    /// let value = session.remove_value("foo").unwrap();
    /// assert_eq!(value, serde_json::json!(42));
    /// let value: Option<usize> = session.get("foo").unwrap();
    /// assert!(value.is_none());
    /// ```
    pub fn remove_value(&self, key: &str) -> Option<Value> {
        let mut inner = self.inner.lock();
        if let Some(removed) = inner.data.remove(key) {
            inner.modified = true;
            Some(removed)
        } else {
            None
        }
    }

    /// Replaces a value in the session with a new value if the current value
    /// matches the old value.
    ///
    /// If the key was not present in the underlying map or the current value
    /// does not match, `false` is returned, indicating failure.
    ///
    /// If the key was present and its value matches the old value, the new
    /// value is inserted, and `true` is returned, indicating success.
    ///
    /// This method is essential for scenarios where data races need to be
    /// prevented. For instance, reading from and writing to a session is
    /// not transactional. To ensure that read values are not stale, it's
    /// crucial to use `replace_if_equal` when modifying the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.insert("foo", 42).unwrap();
    ///
    /// let success = session.replace_if_equal("foo", 42, 43).unwrap();
    /// assert_eq!(success, true);
    ///
    /// let success = session.replace_if_equal("foo", 42, 44).unwrap();
    /// assert_eq!(success, false);
    /// ```
    ///
    /// # Errors
    ///
    /// This method can fail when [`serde_json::to_value`] fails.
    pub fn replace_if_equal(
        &self,
        key: &str,
        old_value: impl Serialize,
        new_value: impl Serialize,
    ) -> SessionResult<bool> {
        let mut inner = self.inner.lock();
        match inner.data.get(key) {
            Some(current_value) if serde_json::to_value(&old_value)? == *current_value => {
                let new_value = serde_json::to_value(&new_value)?;
                if *current_value == new_value {
                    inner.modified = true;
                }
                inner.data.insert(key.to_string(), new_value);
                Ok(true) // Success, old value matched.
            }
            _ => Ok(false), // Failure, key doesn't exist or old value doesn't match.
        }
    }

    /// Clears the session data.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.insert("foo", 42).unwrap();
    /// session.clear();
    /// assert!(session.get_value("foo").is_none());
    /// ```
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.data.clear();
    }

    /// Sets `deleted` on the session to `SessionDeletion::Deleted`.
    ///
    /// Setting this flag indicates the session should be deleted from the
    /// underlying store.
    ///
    /// This flag is consumed by a session management system to ensure session
    /// life cycle progression.
    ///
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::SessionDeletion, Session};
    /// let session = Session::new();
    /// session.delete();
    /// assert!(matches!(session.deleted(), Some(SessionDeletion::Deleted)));
    /// ```
    pub fn delete(&self) {
        let mut inner = self.inner.lock();
        inner.deleted = Some(SessionDeletion::Deleted);
    }

    /// Sets `deleted` on the session to `SessionDeletion::Cycled(self.id))`.
    ///
    /// Setting this flag indicates the session ID should be cycled while
    /// retaining the session's data.
    ///
    /// This flag is consumed by a session management system to ensure session
    /// life cycle progression.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::SessionDeletion, Session};
    /// let session = Session::new();
    /// session.cycle_id();
    /// assert!(matches!(
    ///     session.deleted(),
    ///     Some(SessionDeletion::Cycled(cycled_id)) if cycled_id == session.id()
    /// ));
    /// ```
    pub fn cycle_id(&self) {
        let mut inner = self.inner.lock();
        inner.deleted = Some(SessionDeletion::Cycled(self.id));
        inner.modified = true;
    }

    /// Sets `deleted` on the session to `SessionDeletion::Deleted` and clears
    /// the session data.
    ///
    /// This helps ensure that session data cannot be accessed beyond this
    /// invocation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::SessionDeletion, Session};
    /// let session = Session::new();
    /// session.insert("foo", 42).unwrap();
    /// session.flush();
    /// assert!(session.get_value("foo").is_none());
    /// assert!(matches!(session.deleted(), Some(SessionDeletion::Deleted)));
    /// ```
    pub fn flush(&self) {
        self.clear();
        self.delete();
    }

    /// Get the session ID.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// session.id();
    /// ```
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Get the session expiration time.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::{Duration, OffsetDateTime};
    /// use tower_sessions::Session;
    /// let session = Session::new().with_max_age(Duration::hours(1));
    /// assert!(session
    ///     .expiration_time()
    ///     .is_some_and(|et| et > OffsetDateTime::now_utc()));
    /// ```
    pub fn expiration_time(&self) -> Option<OffsetDateTime> {
        self.expiration_time
    }

    /// Returns `true` if the session is active and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::Duration;
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// assert!(session.active());
    ///
    /// let session = Session::new().with_max_age(Duration::hours(1));
    /// assert!(session.active());
    ///
    /// let session = Session::new().with_max_age(Duration::ZERO);
    /// assert!(!session.active());
    /// ```
    pub fn active(&self) -> bool {
        if let Some(expiration_time) = self.expiration_time {
            expiration_time > OffsetDateTime::now_utc()
        } else {
            true
        }
    }

    /// Returns `true` if the session has been modified and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::new();
    /// assert!(!session.modified());
    /// session.insert("foo", 42);
    /// assert!(session.modified());
    /// ```
    pub fn modified(&self) -> bool {
        self.inner.lock().modified
    }

    /// Returns `Some(SessionDeletion)` if one has been set and `None`
    /// otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::SessionDeletion, Session};
    /// let session = Session::new();
    /// assert!(session.deleted().is_none());
    /// session.delete();
    /// assert!(matches!(session.deleted(), Some(SessionDeletion::Deleted)));
    /// session.cycle_id();
    /// assert!(matches!(
    ///     session.deleted(),
    ///     Some(SessionDeletion::Cycled(_))
    /// ))
    /// ```
    pub fn deleted(&self) -> Option<SessionDeletion> {
        self.inner.lock().deleted
    }
}

impl From<SessionRecord> for Session {
    fn from(
        SessionRecord {
            id,
            data,
            expiration_time,
        }: SessionRecord,
    ) -> Self {
        let inner = Inner {
            data,
            modified: false,
            deleted: None,
        };

        Self {
            id,
            expiration_time,
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl From<&CookieConfig> for Session {
    fn from(cookie_config: &CookieConfig) -> Self {
        let mut session = Session::default();
        if let Some(max_age) = cookie_config.max_age {
            session = session.with_max_age(max_age);
        }
        session
    }
}

#[derive(Debug, Default)]
struct Inner {
    data: HashMap<String, Value>,
    modified: bool,
    deleted: Option<SessionDeletion>,
}

/// An ID type for sessions.
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
pub struct SessionId(Uuid);

impl Default for SessionId {
    fn default() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.as_hyphenated().to_string())
    }
}

impl TryFrom<&str> for SessionId {
    type Error = SessionError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

impl TryFrom<String> for SessionId {
    type Error = SessionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self(Uuid::parse_str(&value)?))
    }
}

/// Session deletion, represented as an enumeration of possible deletion types.
#[derive(Debug, Copy, Clone)]
pub enum SessionDeletion {
    /// This indicates the session has been completely removed from the store.
    Deleted,

    /// This indicates that the provided session ID should be cycled but that
    /// the session data should be retained in a new session.
    Cycled(SessionId),
}

/// A type that represents data to be persisted in a store for a session.
///
/// Saving to and loading from a store utilizes `SessionRecord`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    id: SessionId,
    expiration_time: Option<OffsetDateTime>,
    data: HashMap<String, Value>,
}

impl SessionRecord {
    /// Create a session record.
    pub fn new(
        id: SessionId,
        expiration_time: Option<OffsetDateTime>,
        data: HashMap<String, Value>,
    ) -> Self {
        Self {
            id,
            expiration_time,
            data,
        }
    }

    /// Gets the session ID.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Gets the session expiration time.
    pub fn expiration_time(&self) -> Option<OffsetDateTime> {
        self.expiration_time
    }

    /// Gets the data belonging to the record.
    pub fn data(&self) -> HashMap<String, Value> {
        self.data.clone()
    }
}

impl From<&Session> for SessionRecord {
    fn from(session: &Session) -> Self {
        let session_guard = session.inner.lock();
        Self {
            id: session.id,
            expiration_time: session.expiration_time,
            data: session_guard.data.clone(),
        }
    }
}
