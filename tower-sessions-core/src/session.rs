//! A session which allows HTTP applications to associate data with visitors.
use std::{
    fmt::{self, Debug, Display},
    marker::PhantomData,
    str,
};
// TODO: Remove send + sync bounds on `R` once return type notation is stable.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};

use crate::SessionStore;

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
