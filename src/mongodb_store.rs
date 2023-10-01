use std::collections::HashMap;

use async_trait::async_trait;
use bson::{doc, to_document, DateTime};
use mongodb::{options::UpdateOptions, Client, Collection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::{
    session::{SessionId, SessionRecord},
    Session, SessionStore,
};

/// An error type for `MongoDBStore`.
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

#[derive(Serialize, Deserialize, Debug)]
struct MongoDBSessionRecord {
    data: HashMap<String, Value>,

    #[serde(rename = "expireAt")]
    expiration_time: Option<DateTime>,
}

/// A MongoDB session store.
#[derive(Clone, Debug)]
pub struct MongoDBStore {
    collection: Collection<MongoDBSessionRecord>,
}

impl MongoDBStore {
    /// Create a new MongoDBStore store with the provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{mongodb::Client, MongoDBStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let client = Client::with_uri_str(database_url).await.unwrap();
    /// let session_store = MongoDBStore::new(client, "database".to_string());
    /// # })
    /// ```
    pub fn new(client: Client, database: String) -> Self {
        Self {
            collection: client.database(&database).collection("sessions"),
        }
    }

    /// This function will keep running indefinitely, deleting expired documents
    /// and then waiting for the specified period before deleting again.
    ///
    /// Generally this will be used as a task, for example via
    /// `tokio::task::spawn`.
    ///
    /// # Arguments
    ///
    /// * `period` - The interval at which expired rows should be deleted.
    ///
    /// # Errors
    ///
    /// This function returns a `Result` that contains an error of type
    /// `mongodb::error::Error` if the deletion operation fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{mongodb::Client, MongoDBStore, PostgresStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let client = Client::with_uri_str(database_url).await.unwrap();
    /// let session_store = MongoDBStore::new(client, "database".to_string());
    ///
    /// tokio::task::spawn(
    ///     session_store
    ///         .clone()
    ///         .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    /// );
    /// # })
    /// ```
    #[cfg(feature = "tokio")]
    pub async fn continuously_delete_expired(
        self,
        period: tokio::time::Duration,
    ) -> Result<(), mongodb::error::Error> {
        let mut interval = tokio::time::interval(period);
        loop {
            self.delete_expired().await?;
            interval.tick().await;
        }
    }

    async fn delete_expired(&self) -> mongodb::error::Result<()> {
        self.collection
            .delete_many(
                doc! { "expireAt": {"$lt": OffsetDateTime::now_utc()} },
                None,
            )
            .await?;

        Ok(())
    }
}

#[async_trait]
impl SessionStore for MongoDBStore {
    type Error = MongoDBStoreError;

    async fn save(&self, session_record: &SessionRecord) -> Result<(), Self::Error> {
        self.collection
            .update_one(
                doc! {
                    "_id": session_record.id().to_string()
                },
                doc! {
                    "$set": to_document(&MongoDBSessionRecord {
                        data: session_record.data().clone(),
                        expiration_time: session_record.expiration_time().map(DateTime::from),
                    })?,
                },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        Ok(self
            .collection
            .find_one(
                doc! { "_id": session_id.to_string(), "$or": [
                    {"expireAt": {"$eq": null}},
                    {"expireAt": {"$gt": OffsetDateTime::now_utc()}}
                ] },
                None,
            )
            .await?
            .map(|record| {
                SessionRecord::new(
                    *session_id,
                    record.expiration_time.map(Into::into),
                    record.data,
                )
            })
            .map(Into::into))
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.collection
            .delete_one(doc! { "_id": session_id.to_string() }, None)
            .await?;

        Ok(())
    }
}
