use std::collections::HashMap;

use async_trait::async_trait;
use bson::{doc, to_document, DateTime};
use mongodb::{options::UpdateOptions, Client, Collection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

/// An error type for `MongoDBStore`.
#[allow(clippy::enum_variant_names)]
#[derive(thiserror::Error, Debug)]
pub enum MongoDBStoreError {
    /// A variant to map to `mongodb::error::Error` errors.
    #[error("MongoDB error: {0}")]
    MongoDB(#[from] mongodb::error::Error),

    /// A variant to map `mongodb::bson` encode errors.
    #[error("Bson serialize error: {0}")]
    BsonSerialize(#[from] bson::ser::Error),

    /// A variant to map `mongodb::bson` decode errors.
    #[error("Bson deserialize error: {0}")]
    BsonDeserialize(#[from] bson::de::Error),
}

/// A MongoDB session store.
#[derive(Clone, Debug)]
pub struct MongoDBStore {
    client: Client,
    database: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct MongoDBSessionRecord {
    expiration_time: Option<DateTime>,
    data: HashMap<String, Value>,
}

impl MongoDBStore {
    /// Create a new MongoDBStore store with the provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mongodb::Client;
    /// use tower_sessions::MongoDBStore;
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let client = Client::with_uri_str(database_url).await.unwrap();
    ///
    /// let session_store = MongoDBStore::new(client, "database".to_string());
    /// # })
    /// ```
    pub fn new(client: Client, database: String) -> Self {
        Self { client, database }
    }

    fn col(&self) -> Collection<MongoDBSessionRecord> {
        self.client.database(&self.database).collection("sessions")
    }
}

#[async_trait]
impl SessionStore for MongoDBStore {
    type Error = MongoDBStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        self.col()
            .update_one(
                doc! {
                    "_id": session_record.id().as_uuid()
                },
                doc! {
                    "$set": to_document(&MongoDBSessionRecord {
                        expiration_time: session_record.expiration_time().map(DateTime::from),
                        data: session_record.data().clone(),
                    })?,
                },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let uuid = session_id.as_uuid();

        Ok(self
            .col()
            .find_one(doc! { "_id": uuid }, None)
            .await?
            .map(|record| {
                SessionRecord::new(
                    *session_id,
                    record
                        .expiration_time
                        .map(|expiration_time| expiration_time.into()),
                    record.data,
                )
            })
            .map(Into::into))
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.col()
            .delete_one(doc! { "_id": session_id.to_string() }, None)
            .await?;

        Ok(())
    }
}
