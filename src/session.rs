//! A session which allows HTTP applications to associate data with visitors.
use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Display,
    hash::{Hash, Hasher},
    sync::Arc,
};

use parking_lot::Mutex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use time::Duration;
use tower_cookies::cookie::time::OffsetDateTime;
use uuid::Uuid;

const DEFAULT_DURATION: Duration = Duration::weeks(2);

/// Session errors.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// A variant to map `uuid` errors.
    #[error("Invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    /// A variant to map `serde_json` errors.
    #[error("JSON serialization/deserialization error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, Error>;

type Data = HashMap<String, Value>;

/// A session which allows HTTP applications to associate key-value pairs with
/// visitors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub(crate) id: Id,
    inner: Arc<Mutex<Inner>>,
}

impl Session {
    /// Create a new session with the given expiry.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::{Duration, OffsetDateTime};
    /// use tower_sessions::{Expiry, Session};
    ///
    /// // Uses a so-called "session cookie".
    /// let expiry = Expiry::OnSessionEnd;
    /// Session::new(Some(expiry));
    ///
    /// // Uses an inactivity expiry.
    /// let expiry = Expiry::OnInactivity(Duration::hours(1));
    /// Session::new(Some(expiry));
    ///
    /// // Uses a date and time expiry.
    /// let expired_at = OffsetDateTime::now_utc().saturating_add(Duration::hours(1));
    /// let expiry = Expiry::AtDateTime(expired_at);
    /// Session::new(Some(expiry));
    /// ```
    pub fn new(expiry: Option<Expiry>) -> Self {
        let inner = Inner {
            expiry,
            ..Default::default()
        };

        Self {
            id: Id::default(),
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Inserts a `impl Serialize` value into the session.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::default();
    /// session.insert("foo", 42).expect("Serialization error.");
    /// ```
    ///
    /// # Errors
    ///
    /// This method can fail when [`serde_json::to_value`] fails.
    pub fn insert(&self, key: &str, value: impl Serialize) -> Result<()> {
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
    /// let session = Session::default();
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
            inner.modified_at = Some(OffsetDateTime::now_utc());
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
    /// let session = Session::default();
    /// session.insert("foo", 42).unwrap();
    /// let value = session.get::<usize>("foo").unwrap();
    /// assert_eq!(value, Some(42));
    /// ```
    ///
    /// # Errors
    ///
    /// This method can fail when [`serde_json::from_value`] fails.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
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
    /// let session = Session::default();
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
    /// let session = Session::default();
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
    pub fn remove<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
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
    /// let session = Session::default();
    /// session.insert("foo", 42).unwrap();
    /// let value = session.remove_value("foo").unwrap();
    /// assert_eq!(value, serde_json::json!(42));
    /// let value: Option<usize> = session.get("foo").unwrap();
    /// assert!(value.is_none());
    /// ```
    pub fn remove_value(&self, key: &str) -> Option<Value> {
        let mut inner = self.inner.lock();
        if let Some(removed) = inner.data.remove(key) {
            inner.modified_at = Some(OffsetDateTime::now_utc());
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
    /// let session = Session::default();
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
    ) -> Result<bool> {
        let mut inner = self.inner.lock();
        match inner.data.get(key) {
            Some(current_value) if serde_json::to_value(&old_value)? == *current_value => {
                let new_value = serde_json::to_value(&new_value)?;
                if *current_value != new_value {
                    inner.modified_at = Some(OffsetDateTime::now_utc());
                    inner.data.insert(key.to_string(), new_value);
                }
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
    /// let session = Session::default();
    /// session.insert("foo", 42).unwrap();
    /// session.clear();
    /// assert!(session.get_value("foo").is_none());
    /// ```
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.data.clear();
    }

    /// Sets `deleted` on the session to `Deletion::Deleted`.
    ///
    /// Setting this flag indicates the session should be deleted from the
    /// underlying store.
    ///
    /// This flag is consumed by a session management system to ensure session
    /// life cycle progression.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::Deletion, Session};
    /// let session = Session::default();
    /// session.delete();
    /// assert!(matches!(session.deleted(), Some(Deletion::Deleted)));
    /// ```
    pub fn delete(&self) {
        let mut inner = self.inner.lock();
        inner.deleted = Some(Deletion::Deleted);
    }

    /// Sets `deleted` on the session to `Deletion::Cycled(self.id))`.
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
    /// use tower_sessions::{session::Deletion, Session};
    /// let session = Session::default();
    /// session.insert("foo", 42);
    /// session.cycle_id();
    /// assert!(matches!(
    ///     session.deleted(),
    ///     Some(Deletion::Cycled(ref cycled_id)) if cycled_id == session.id()
    /// ));
    /// ```
    pub fn cycle_id(&self) {
        let mut inner = self.inner.lock();
        inner.deleted = Some(Deletion::Cycled(self.id));
        inner.modified_at = Some(OffsetDateTime::now_utc());
    }

    /// Sets `deleted` on the session to `Deletion::Deleted` and clears
    /// the session data.
    ///
    /// This helps ensure that session data cannot be accessed beyond this
    /// invocation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::Deletion, Session};
    /// let session = Session::default();
    /// session.insert("foo", 42).unwrap();
    /// session.flush();
    /// assert!(session.get_value("foo").is_none());
    /// assert!(matches!(session.deleted(), Some(Deletion::Deleted)));
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
    /// let session = Session::default();
    /// session.id();
    /// ```
    pub fn id(&self) -> &Id {
        &self.id
    }

    /// Get the session expiry.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::{Duration, OffsetDateTime};
    /// use tower_sessions::{Expiry, Session};
    ///
    /// let expiry = Expiry::OnInactivity(Duration::hours(1));
    /// let session = Session::default();
    /// session.set_expiry(Some(expiry));
    /// assert_eq!(
    ///     session.expiry(),
    ///     Some(Expiry::OnInactivity(Duration::hours(1)))
    /// );
    /// ```
    pub fn expiry(&self) -> Option<Expiry> {
        let inner = self.inner.lock();
        inner.expiry.clone()
    }

    /// Set `expiration_time` give the given value.
    ///
    /// This may be used within applications directly to alter the session's
    /// time to live.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use time::{Duration, OffsetDateTime};
    /// use tower_sessions::{Expiry, Session};
    ///
    /// let session = Session::default();
    /// let expiry = Expiry::AtDateTime(OffsetDateTime::from_unix_timestamp(0).unwrap());
    /// session.set_expiry(Some(expiry));
    /// session.insert("foo", 42);
    /// assert!(session.is_modified());
    ///
    /// let expiry = Expiry::OnInactivity(Duration::weeks(2));
    /// session.set_expiry(Some(expiry));
    /// assert!(session.is_modified());
    /// ```
    pub fn set_expiry(&self, expiry: Option<Expiry>) {
        let mut inner = self.inner.lock();
        inner.expiry = expiry;
        inner.modified_at = Some(OffsetDateTime::now_utc());
    }

    /// Get session expiry as `OffsetDateTime`.
    pub fn expiry_date(&self) -> OffsetDateTime {
        let inner = self.inner.lock();
        match inner.expiry {
            Some(Expiry::OnInactivity(duration)) => {
                let modified_at = inner.modified_at.unwrap_or_else(OffsetDateTime::now_utc);
                modified_at.saturating_add(duration)
            }
            Some(Expiry::AtDateTime(datetime)) => datetime,
            Some(Expiry::OnSessionEnd) | None => {
                // TODO: The default should probably be configurable.
                OffsetDateTime::now_utc().saturating_add(DEFAULT_DURATION)
            }
        }
    }

    /// Get session expiry as `Duration`.
    pub fn expiry_age(&self) -> Duration {
        self.expiry_date() - OffsetDateTime::now_utc()
    }

    /// Returns `true` if the session has been modified and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    ///
    /// let session = Session::default();
    /// assert!(!session.is_modified());
    ///
    /// session.insert("foo", 42);
    /// assert!(session.is_modified());
    /// ```
    pub fn is_modified(&self) -> bool {
        let inner = self.inner.lock();
        inner.modified_at.is_some() && !inner.data.is_empty()
    }

    /// Returns `Some(Deletion)` if one has been set and `None`
    /// otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::{session::Deletion, Session};
    /// let session = Session::default();
    /// session.insert("foo", 42);
    /// assert!(session.deleted().is_none());
    /// session.delete();
    /// assert!(matches!(session.deleted(), Some(Deletion::Deleted)));
    /// session.cycle_id();
    /// assert!(matches!(session.deleted(), Some(Deletion::Cycled(_))))
    /// ```
    pub fn deleted(&self) -> Option<Deletion> {
        // Empty sessions are deleted to ensure removal of the last key.
        if self.is_empty() {
            return Some(Deletion::Deleted);
        };

        self.inner.lock().deleted
    }

    /// Returns `true` if the session is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tower_sessions::Session;
    /// let session = Session::default();
    /// assert!(session.is_empty());
    ///
    /// session.insert("foo", 42);
    /// assert!(!session.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.inner.lock().data.is_empty()
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Session {}

impl Hash for Session {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

impl Borrow<Id> for Session {
    fn borrow(&self) -> &Id {
        self.id()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Inner {
    data: Data,
    expiry: Option<Expiry>,
    modified_at: Option<OffsetDateTime>,
    deleted: Option<Deletion>,
}

/// An ID type for sessions.
///
/// # Examples
///
/// ```rust
/// use tower_sessions::session::Id;
/// let session_id = Id::default();
/// ```
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
pub struct Id(pub Uuid);

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

impl TryFrom<&str> for Id {
    type Error = Error;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

impl TryFrom<String> for Id {
    type Error = Error;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Ok(Self(Uuid::parse_str(&value)?))
    }
}

/// Session deletion, represented as an enumeration of possible deletion types.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum Deletion {
    /// This indicates the session has been completely removed from the store.
    Deleted,

    /// This indicates that the provided session ID should be cycled but that
    /// the session data should be retained in a new session.
    Cycled(Id),
}

/// Session expiry configuration.
///
/// # Examples
///
/// ```rust
/// use time::{Duration, OffsetDateTime};
/// use tower_sessions::Expiry;
///
/// let expiry = Expiry::OnInactivity(Duration::minutes(5));
///
/// let expired_at = OffsetDateTime::now_utc().saturating_add(Duration::minutes(5));
/// let expiry = Expiry::AtDateTime(expired_at);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Expiry {
    /// Expire on [current session end][current-session-end], as defined by the
    /// browser.
    ///
    /// [current-session-end]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Cookies#define_the_lifetime_of_a_cookie
    OnSessionEnd,

    /// Expire on inactivity.
    OnInactivity(Duration),

    /// Expire at a specific date and time.
    AtDateTime(OffsetDateTime),
}
