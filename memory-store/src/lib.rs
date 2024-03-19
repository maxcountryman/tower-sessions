use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use tower_sessions_core::{
    session::{Id, Record},
    session_store, SessionStore,
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
#[derive(Clone, Debug, Default)]
pub struct MemoryStore(Arc<Mutex<HashMap<Id, Record>>>);

#[async_trait]
impl SessionStore for MemoryStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        let mut store_guard = self.0.lock().await;
        while store_guard.contains_key(&record.id) {
            // Session ID collision mitigation.
            record.id = Id::default();
        }
        store_guard.insert(record.id, record.clone());
        Ok(())
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        self.0.lock().await.insert(record.id, record.clone());
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        Ok(self
            .0
            .lock()
            .await
            .get(session_id)
            .filter(|Record { expiry_date, .. }| is_active(*expiry_date))
            .cloned())
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        self.0.lock().await.remove(session_id);
        Ok(())
    }
}

fn is_active(expiry_date: OffsetDateTime) -> bool {
    expiry_date > OffsetDateTime::now_utc()
}
