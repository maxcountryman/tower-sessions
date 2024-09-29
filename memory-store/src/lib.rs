use std::{collections::HashMap, convert::Infallible, sync::Arc};

use time::OffsetDateTime;
use tokio::sync::Mutex;
use tower_sessions_core::{Id, SessionStore};
use std::fmt::Debug;

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
pub struct MemoryStore<R>(Arc<Mutex<HashMap<Id, R>>>);

impl<R> SessionStore<R> for MemoryStore<R>
where
    R: Send + Sync + Debug + Clone,
{
    type Error = Infallible;

    async fn create(
        &mut self,
        record: &R,
    ) -> Result<Id, Self::Error> {
        let mut id = random_id();
        let mut store = self.0.lock().await;
        while store.contains_key(&id) {
            // If the ID already exists, generate a new one
            id = random_id();
        }
        store.insert(id, record.clone());
        Ok(id)
    }

    async fn save(
        &mut self,
        id: &Id,
        record: &R,
    ) -> Result<bool, Self::Error> {
        let mut store = self.0.lock().await;
        if store.contains_key(id) {
            store.insert(*id, record.clone());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn save_or_create(
        &mut self,
        id: &Id,
        record: &R,
    ) -> Result<(), Self::Error> {
        let mut store = self.0.lock().await;
        store.insert(*id, record.clone());
        Ok(())
    }

    async fn load(
        &mut self,
        id: &Id,
    ) -> Result<Option<R>, Self::Error> {
        let store = self.0.lock().await;
        Ok(store.get(id).cloned())
    }

    async fn delete(&mut self, id: &Id) -> Result<bool, Self::Error> {
        let mut store = self.0.lock().await;
        Ok(store.remove(id).is_some())
    }

    async fn cycle_id(
        &mut self,
        old_id: &Id,
    ) -> Result<Option<Id>, Self::Error> {
        let mut store = self.0.lock().await;
        if let Some(record) = store.remove(old_id) {
            let mut new_id = random_id();
            while store.contains_key(&new_id) {
                // If the ID already exists, generate a new one
                new_id = random_id();
            }
            store.insert(new_id, record);
            Ok(Some(new_id))
        } else {
            Ok(None)
        }
    }
}

fn is_active(expiry_date: OffsetDateTime) -> bool {
    expiry_date > OffsetDateTime::now_utc()
}

fn random_id() -> Id {
    use rand::prelude::*;
    let id_val = rand::thread_rng().gen();
    Id(id_val)
}

#[cfg(test)]
mod tests {
    use time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_create() {
        let store = MemoryStore::default();
        let mut record = Record {
            id: Default::default(),
            data: Default::default(),
            expiry_date: OffsetDateTime::now_utc() + Duration::minutes(30),
        };
        assert!(store.create(&mut record).await.is_ok());
    }

    #[tokio::test]
    async fn test_save() {
        let store = MemoryStore::default();
        let record = Record {
            id: Default::default(),
            data: Default::default(),
            expiry_date: OffsetDateTime::now_utc() + Duration::minutes(30),
        };
        assert!(store.save(&record).await.is_ok());
    }

    #[tokio::test]
    async fn test_load() {
        let store = MemoryStore::default();
        let mut record = Record {
            id: Default::default(),
            data: Default::default(),
            expiry_date: OffsetDateTime::now_utc() + Duration::minutes(30),
        };
        store.create(&mut record).await.unwrap();
        let loaded_record = store.load(&record.id).await.unwrap();
        assert_eq!(Some(record), loaded_record);
    }

    #[tokio::test]
    async fn test_delete() {
        let store = MemoryStore::default();
        let mut record = Record {
            id: Default::default(),
            data: Default::default(),
            expiry_date: OffsetDateTime::now_utc() + Duration::minutes(30),
        };
        store.create(&mut record).await.unwrap();
        assert!(store.delete(&record.id).await.is_ok());
        assert_eq!(None, store.load(&record.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_create_id_collision() {
        let store = MemoryStore::default();
        let expiry_date = OffsetDateTime::now_utc() + Duration::minutes(30);
        let mut record1 = Record {
            id: Default::default(),
            data: Default::default(),
            expiry_date,
        };
        let mut record2 = Record {
            id: Default::default(),
            data: Default::default(),
            expiry_date,
        };
        store.create(&mut record1).await.unwrap();
        record2.id = record1.id; // Set the same ID for record2
        store.create(&mut record2).await.unwrap();
        assert_ne!(record1.id, record2.id); // IDs should be different
    }
}
