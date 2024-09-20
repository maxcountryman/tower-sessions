//! A session which allows HTTP applications to associate data with visitors.
use std::{
    fmt::{self, Debug, Display},
    hash::Hash,
    marker::PhantomData,
    result,
    str::{self, FromStr},
};
// TODO: Remove send + sync bounds on `R` once return type notation is stable.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, DecodeError, Engine as _};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::SessionStore;

const DEFAULT_DURATION: Duration = Duration::weeks(2);

/// A session which allows HTTP applications to associate key-value pairs with
/// visitors.
pub struct Session<R, Store> {
    pub store: Store,
    /// This will be `None` if the endpoint has not received a session cookie or if the it could
    /// not be parsed.
    pub id: Option<Id>,
    data: PhantomData<R>,
}

impl<R, Store: Clone> Clone for Session<R, Store> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            id: self.id,
            data: PhantomData,
        }
    }
}

impl<R, Store: Debug> Debug for Session<R, Store> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("store", &self.store)
            .field("id", &self.id)
            .field("data", &self.data)
            .finish()
    }
}

impl<R: Send + Sync, Store: SessionStore<R>> Session<R, Store> {
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
    pub fn new(store: Store, id: Option<Id>) -> Self {
        Self {
            store,
            id,
            data: Default::default(),
        }
    }

    /// Load the session data from the store.
    ///
    /// This method will return `None` if the session `Id` for this session is `None`,
    /// or if the session could not be loaded from the store (the session does not exist).
    ///
    /// Otherwise, this method can error if the underlying store fails to load the session.
    ///
    /// If all goes well, this session returns a `LoadedSession` which can be used to interact with
    /// the session data.
    ///
    /// ### Examples
    /// ```rust
    /// todo!()
    /// ```
    pub async fn load(self) -> Result<Option<LoadedSession<R, Store>>, Store::Error> {
        let Some(session_id) = self.id else {
            return Ok(None);
        };

        Ok(self
            .store
            .load(&session_id)
            .await?
            .map(|data| LoadedSession {
                store: self.store,
                id: session_id,
                data,
            }))
    }
}

pub struct LoadedSession<R, Store> {
    pub store: Store,
    pub id: Id,
    pub data: R,
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
    /// [current-session-end]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Cookies#removal_defining_the_lifetime_of_a_cookie
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

impl Expiry {
    /// Get expiry as `OffsetDateTime`.
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
        match self {
            Expiry::OnInactivity(duration) => OffsetDateTime::now_utc().saturating_add(*duration),
            Expiry::AtDateTime(datetime) => *datetime,
            Expiry::OnSessionEnd => {
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
