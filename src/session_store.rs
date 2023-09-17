//! An arbitrary store which houses the session data.
use async_trait::async_trait;

use crate::session::{Session, SessionId, SessionRecord};

/// An arbitrary store which houses the session data.
#[async_trait]
pub trait SessionStore: Clone + Send + Sync + 'static {
    /// An error that occurs when interacting with the store.
    type Error: std::error::Error + Send + Sync;

    /// A method for saving a session in a store.
    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error>;

    /// A method for loading a session from a store.
    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error>;

    /// A method for deleting a session from a store.
    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error>;
}
