use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

/// An error type for `MemoryStore`.
#[derive(thiserror::Error, Debug)]
pub enum MemoryStoreError {
    /// A variant to map `serde_json` errors.
    #[error("JSON serialization/deserialization error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
}

/// A session store that lives only in memory.
///
/// This is useful for testing but not recommended for real applications.
///
/// # Examples
///
/// ```rust
/// use tower_sessions::MemoryStore;
/// MemoryStore::default();
/// ```
#[derive(Clone, Default)]
pub struct MemoryStore(Arc<Mutex<HashMap<SessionId, Value>>>);

#[async_trait]
impl SessionStore for MemoryStore {
    type Error = MemoryStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        self.0.lock().insert(
            session_record.id(),
            serde_json::to_value(session_record).map_err(MemoryStoreError::from)?,
        );
        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let session = if let Some(record_value) = self.0.lock().get(session_id) {
            let session_record: SessionRecord =
                serde_json::from_value(record_value.clone()).map_err(MemoryStoreError::from)?;
            Some(session_record.into())
        } else {
            None
        };
        Ok(session)
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.0.lock().remove(session_id);
        Ok(())
    }
}
