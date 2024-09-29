use std::{collections::HashMap, convert::Infallible, sync::Arc};

use std::fmt::Debug;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use tower_sessions_core::{expires::Expires, Expiry, Id, SessionStore};

/// A session store that lives only in memory.
///
/// This is useful for testing but not recommended for real applications.
///
/// The store manages the expiry of the sessions with respect to UTC time. No cleanup is done for
/// the expired sessions untile the are loaded.
///
/// # Examples
///
/// ```rust
/// use tower_sessions_memory_store::MemoryStore;
///
/// struct User {
///    name: String,
///    age: u8,
/// }
/// 
/// let store: MemoryStore<User> = MemoryStore::default();
/// ```
#[derive(Clone, Debug)]
pub struct MemoryStore<R>(Arc<Mutex<HashMap<Id, Value<R>>>>);

impl<R> Default for MemoryStore<R> {
    fn default() -> Self {
        MemoryStore(Default::default())
    }
}

#[derive(Debug, Clone)]
struct Value<R> {
    data: R,
    // Needed because if the expiry date is set to `OnInactivity`, we need to know whether the
    // session is active or not.
    expiry_date: Option<OffsetDateTime>,
}

impl<R: Expires> Value<R> {
    /// Create a new `MemoryStore`.
    pub fn new(data: R) -> Self {
        let expiry_date = match data.expires() {
            Expiry::OnSessionEnd => None,
            Expiry::OnInactivity(duration) => Some(OffsetDateTime::now_utc() + duration),
            Expiry::AtDateTime(offset_date_time) => Some(offset_date_time),
        };

        Value { data, expiry_date }
    }
}

impl<R> SessionStore<R> for MemoryStore<R>
where
    R: Expires + Send + Sync + Debug + Clone,
{
    type Error = Infallible;

    async fn create(&mut self, record: &R) -> Result<Id, Self::Error> {
        let mut id = random_id();
        let mut store = self.0.lock().await;
        while store.contains_key(&id) {
            // If the ID already exists, generate a new one
            id = random_id();
        }

        let value = Value::new(record.clone());

        store.insert(id, value);
        Ok(id)
    }

    async fn save(&mut self, id: &Id, record: &R) -> Result<bool, Self::Error> {
        let mut store = self.0.lock().await;
        if store.contains_key(id) {
            let value = Value::new(record.clone());
            store.insert(*id, value);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn save_or_create(&mut self, id: &Id, record: &R) -> Result<(), Self::Error> {
        let mut store = self.0.lock().await;
        let value = Value::new(record.clone());
        store.insert(*id, value);
        Ok(())
    }

    async fn load(&mut self, id: &Id) -> Result<Option<R>, Self::Error> {
        let mut store = self.0.lock().await;

        let Some(value) = store.get(id) else {
            return Ok(None);
        };
        Ok(match value.expiry_date {
            Some(expiry_date) if expiry_date > OffsetDateTime::now_utc() => {
                store.remove(id);
                None
            }
            _ => Some(value.data.clone()),
        })
    }

    async fn delete(&mut self, id: &Id) -> Result<bool, Self::Error> {
        let mut store = self.0.lock().await;
        Ok(store.remove(id).is_some())
    }

    async fn cycle_id(&mut self, old_id: &Id) -> Result<Option<Id>, Self::Error> {
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

fn random_id() -> Id {
    use rand::prelude::*;
    let id_val = rand::thread_rng().gen();
    Id(id_val)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_sessions_core::SessionStore;

    
    #[derive(Debug, Clone)]
    struct SimpleUser {
        age: u8,
    }

    impl Expires for SimpleUser {}

    #[tokio::test]
    async fn round_trip() {
        let mut store: MemoryStore<SimpleUser> = MemoryStore::default();

        let id = store.create(&SimpleUser {
            age: 20,
        }).await.unwrap();

        let mut user = store.load(&id).await.unwrap().unwrap();
        assert_eq!(20, user.age);

        user.age = 30;
        assert!(store.save(&id, &user).await.unwrap());

        let user = store.load(&id).await.unwrap().unwrap();
        assert_eq!(30, user.age);

        let new_id = store.cycle_id(&id).await.unwrap().unwrap();
        assert_ne!(id, new_id);

        assert!(store.load(&id).await.unwrap().is_none());
        let user = store.load(&new_id).await.unwrap().unwrap();
        assert_eq!(30, user.age);

        assert!(store.delete(&new_id).await.unwrap());
        assert!(store.load(&new_id).await.unwrap().is_none());
    }
}
