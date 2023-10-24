use async_trait::async_trait;
use bson::{doc, to_document};
use mongodb::{options::UpdateOptions, Client, Collection};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{session::SessionId, ExpiredDeletion, Session, SessionStore};

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

    /// A variant to map `rmp_serde` encode errors.
    #[error("Rust MsgPack encode error: {0}")]
    RmpSerdeEncode(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error("Rust MsgPack decode error: {0}")]
    RmpSerdeDecode(#[from] rmp_serde::decode::Error),
}

#[derive(Serialize, Deserialize, Debug)]
struct MongoDBSessionRecord {
    data: bson::Binary,

    #[serde(rename = "expireAt")]
    expiry_date: bson::DateTime,
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
}

#[async_trait]
impl ExpiredDeletion for MongoDBStore {
    async fn delete_expired(&self) -> Result<(), Self::Error> {
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

    async fn save(&self, session: &Session) -> Result<(), Self::Error> {
        let doc = to_document(&MongoDBSessionRecord {
            data: bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: rmp_serde::to_vec(session)?,
            },
            expiry_date: bson::DateTime::from(session.expiry_date()),
        })?;

        self.collection
            .update_one(
                doc! { "_id": session.id().to_string() },
                doc! { "$set": doc },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;

        Ok(())
    }

    async fn load(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error> {
        let doc = self
            .collection
            .find_one(
                doc! {
                    "_id": session_id.to_string(),
                    "expireAt": {"$gt": OffsetDateTime::now_utc()}
                },
                None,
            )
            .await?;

        if let Some(doc) = doc {
            Ok(Some(rmp_serde::from_slice(&doc.data.bytes)?))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Self::Error> {
        self.collection
            .delete_one(doc! { "_id": session_id.to_string() }, None)
            .await?;

        Ok(())
    }
}
