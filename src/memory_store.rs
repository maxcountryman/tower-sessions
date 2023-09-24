use std::{collections::HashMap, convert::Infallible, sync::Arc};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

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
pub struct MemoryStore(Arc<Mutex<HashMap<SessionId, SessionRecord>>>);

#[async_trait]
impl SessionStore for MemoryStore {
    type Error = Infallible;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        self.0
            .lock()
            .insert(session_record.id(), session_record.clone());
        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let session = self
            .0
            .lock()
            .get(session_id)
            .map(|session_record| session_record.clone().into());
        Ok(session)
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.0.lock().remove(session_id);
        Ok(())
    }
}
