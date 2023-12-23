use async_trait::async_trait;
pub use aws_config;
pub use aws_sdk_dynamodb;
use aws_sdk_dynamodb::{
    operation::{
        batch_write_item::BatchWriteItemError, delete_item::DeleteItemError,
        put_item::PutItemError, query::QueryError, scan::ScanError,
    },
    primitives::Blob,
    types::{AttributeValue, DeleteRequest, WriteRequest},
    Client,
};
use std::collections::hash_map::HashMap;
use time::OffsetDateTime;
use tower_sessions_core::{
    session::{Id, Record},
    session_store, ExpiredDeletion, SessionStore,
};

/// An error type for `DynamoDBStore`.
#[derive(thiserror::Error, Debug)]
pub enum DynamoDBStoreError {
    /// A variant to map `aws_sdk_dynamodb::error::BuildError` errors.
    #[error(transparent)]
    DynamoDbBuild(#[from] aws_sdk_dynamodb::error::BuildError),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<QueryError>` errors.
    #[error(transparent)]
    DynamoDbQuery(#[from] aws_sdk_dynamodb::error::SdkError<QueryError>),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<PutItemError>` errors.
    #[error(transparent)]
    DynamoDbPutItem(#[from] aws_sdk_dynamodb::error::SdkError<PutItemError>),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<DeleteItemError>` errors.
    #[error(transparent)]
    DynamoDbDeleteItem(#[from] aws_sdk_dynamodb::error::SdkError<DeleteItemError>),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<BatchWriteItemError>` errors.
    #[error(transparent)]
    DynamoDbBatchWriteItem(#[from] aws_sdk_dynamodb::error::SdkError<BatchWriteItemError>),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<ScanError>` errors.
    #[error(transparent)]
    DynamoDbScan(#[from] aws_sdk_dynamodb::error::SdkError<ScanError>),

    /// A variant to map `rmp_serde` encode errors.
    #[error(transparent)]
    Encode(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error(transparent)]
    Decode(#[from] rmp_serde::decode::Error),
}

impl From<DynamoDBStoreError> for session_store::Error {
    fn from(err: DynamoDBStoreError) -> Self {
        match err {
            DynamoDBStoreError::DynamoDbBuild(inner) => {
                session_store::Error::Backend(inner.to_string())
            }
            DynamoDBStoreError::DynamoDbQuery(inner) => {
                session_store::Error::Backend(inner.to_string())
            }
            DynamoDBStoreError::DynamoDbPutItem(inner) => {
                session_store::Error::Backend(inner.to_string())
            }
            DynamoDBStoreError::DynamoDbDeleteItem(inner) => {
                session_store::Error::Backend(inner.to_string())
            }
            DynamoDBStoreError::DynamoDbBatchWriteItem(inner) => {
                session_store::Error::Backend(inner.to_string())
            }
            DynamoDBStoreError::DynamoDbScan(inner) => {
                session_store::Error::Backend(inner.to_string())
            }
            DynamoDBStoreError::Decode(inner) => session_store::Error::Decode(inner.to_string()),
            DynamoDBStoreError::Encode(inner) => session_store::Error::Encode(inner.to_string()),
        }
    }
}

/// Holds the DynamoDB property name of a key, and optionaly a prefix/suffix to add to the session
/// id before saving to DynamoDB.
#[derive(Clone, Debug)]
pub struct DynamoDBStoreKey {
    /// The property name of the key.
    pub name: String,
    /// The optional prefix to add before the session id (useful for singletable designs).
    pub prefix: Option<String>,
    /// The optional suffix to add after the session id (useful for singletable designs).
    pub suffix: Option<String>,
}

impl Default for DynamoDBStoreKey {
    fn default() -> Self {
        DynamoDBStoreKey {
            name: "session_id".to_string(),
            prefix: Some("SESSIONS::TOWER::".to_string()),
            suffix: None,
        }
    }
}

/// Properties for configuring the session store.
#[derive(Clone, Debug)]
pub struct DynamoDBStoreProps {
    /// DynamoDB table name to store sessions in.
    pub table_name: String,

    /// The DynamoDB partition(hash) key to store the session_id at.
    pub partition_key: DynamoDBStoreKey,

    /// The DynamoDB sort(search) key to store the session_id under (useful with singletable designs).
    pub sort_key: Option<DynamoDBStoreKey>,

    /// The property name to hold the expiration time of the session, a unix timestamp in seconds.
    pub expirey_name: String,

    /// The property name to hold the session data blob.
    pub data_name: String,
}

impl Default for DynamoDBStoreProps {
    fn default() -> Self {
        Self {
            table_name: "tower-sessions".to_string(),
            partition_key: DynamoDBStoreKey::default(),
            sort_key: None,
            expirey_name: "expire_at".to_string(),
            data_name: "data".to_string(),
        }
    }
}

/// A DynamoDB backed session store.
#[derive(Clone, Debug)]
pub struct DynamoDBStore {
    /// the aws-sdk DynamoDB client to use when managing towser-sessions.
    pub client: Client,
    /// the DynamoDB backend configuration properties.
    pub props: DynamoDBStoreProps,
}

impl DynamoDBStore {
    /// Create a new DynamoDBStore store with the default store properties.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions::{
    ///     aws_config,
    ///     aws_sdk_dynamodb,
    ///     DynamoDBStore,
    ///     DynamoDBStoreProps,
    /// };
    /// let store_props = DynamoDBStoreProps::default();
    ///
    /// # tokio_test::block_on(async {
    /// let config = aws_config::load_from_env().await;
    /// let client = aws_sdk_dynamodb::Client::new(&config);
    /// let session_store = DynamoDBStore::new(client, store_props);
    /// # })
    /// ```
    pub fn new(client: Client, props: DynamoDBStoreProps) -> Self {
        Self { client, props }
    }

    fn pk<S: ToString>(&self, input: S) -> String {
        format!(
            "{}{}{}",
            self.props
                .partition_key
                .prefix
                .clone()
                .unwrap_or("".to_string()),
            input.to_string(),
            self.props
                .partition_key
                .suffix
                .clone()
                .unwrap_or("".to_string())
        )
    }

    fn sk<S: ToString>(&self, input: S) -> String {
        if let Some(sk) = &self.props.sort_key {
            format!(
                "{}{}{}",
                sk.prefix.clone().unwrap_or("".to_string()),
                input.to_string(),
                sk.suffix.clone().unwrap_or("".to_string())
            )
        } else {
            "".to_string()
        }
    }
}

#[async_trait]
impl ExpiredDeletion for DynamoDBStore {
    // scans are typically to be avoided in dynamodb; the ExpiredDeletion trait assumes a scan is
    // available on the Store; it recommended to use this in conjunction with
    // a dynamo Time to Live setting on the DynamoDB table.
    // A TTL setting will let dynamodb cull expired sessions inbetween delete_expired runs,
    // preventing the scan from returning larger results, taking longer, costing more, and increasing
    // the chance for a failure while batch processing.
    // NOTE: a DynamoDB TTL does not offer an SLA on deleting the columns below 48 hours. While
    // typically items are removed within seconds of their TLL value, they can remain in the table
    // for up to 2 days before AWS removes them.
    // see: https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/TTL.html
    async fn delete_expired(&self) -> session_store::Result<()> {
        let now_sec = OffsetDateTime::now_utc().unix_timestamp();
        let now_av = AttributeValue::N(now_sec.to_string());

        // expression_attribute_names
        let mut projection = "#pk";
        let mut attribute_names = HashMap::new();
        attribute_names.insert("#expire_at".to_string(), self.props.expirey_name.clone());
        attribute_names.insert("#pk".to_string(), self.props.partition_key.name.clone());
        if let Some(sk) = &self.props.sort_key {
            attribute_names.insert("#sk".to_string(), sk.name.clone());
            projection = "#pk, #sk";
        }

        let mut expired_sessions = self
            .client
            .scan()
            .table_name(&self.props.table_name)
            .set_expression_attribute_names(Some(attribute_names))
            .expression_attribute_values(":now", now_av)
            .filter_expression("#expire_at < :now")
            .projection_expression(projection)
            .into_paginator()
            .page_size(25)
            .items()
            .send();

        // batchwriteitem only processes 25 items at a time
        let mut batches: Vec<Vec<WriteRequest>> = Vec::with_capacity(50);
        let mut batch: Vec<WriteRequest> = Vec::with_capacity(25);
        while let Some(session) = expired_sessions.next().await {
            if batch.len() == 25 {
                batches.push(batch);
                batch = Vec::with_capacity(25);
            }
            let delete_keys = session.map_err(DynamoDBStoreError::DynamoDbScan)?.clone();
            let delete_request = DeleteRequest::builder()
                .set_key(Some(delete_keys))
                .build()
                .map_err(DynamoDBStoreError::DynamoDbBuild)?;
            let write_request = WriteRequest::builder()
                .delete_request(delete_request)
                .build();
            batch.push(write_request);
        }
        if !batch.is_empty() {
            batches.push(batch);
        }

        // process each batch of 25 epired sessions
        for delete_batch in batches {
            let mut unprocessed_count = delete_batch.len();
            let mut unprocessed = Some(HashMap::from([(
                self.props.table_name.clone(),
                delete_batch,
            )]));
            while unprocessed_count > 0 {
                let new_unprocessed_items = self
                    .client
                    .batch_write_item()
                    .set_request_items(unprocessed)
                    .send()
                    .await
                    .map_err(DynamoDBStoreError::DynamoDbBatchWriteItem)?
                    .unprocessed_items;
                unprocessed_count = new_unprocessed_items
                    .as_ref()
                    .map(|m| {
                        m.get(&self.props.table_name)
                            .map(|v| v.len())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                unprocessed = new_unprocessed_items;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl SessionStore for DynamoDBStore {
    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let exp_sec = record.expiry_date.unix_timestamp();
        let data_bytes = rmp_serde::to_vec(record).map_err(DynamoDBStoreError::Encode)?;

        let mut item = HashMap::new();
        item.insert(
            self.props.partition_key.name.clone(),
            AttributeValue::S(self.pk(record.id)),
        );
        item.insert(
            self.props.data_name.clone(),
            AttributeValue::B(Blob::new(data_bytes)),
        );
        item.insert(
            self.props.expirey_name.clone(),
            AttributeValue::N(exp_sec.to_string()),
        );
        if let Some(sk) = &self.props.sort_key {
            item.insert(sk.name.clone(), AttributeValue::S(self.sk(record.id)));
        }

        self.client
            .put_item()
            .table_name(&self.props.table_name)
            .set_item(Some(item))
            .send()
            .await
            .map_err(DynamoDBStoreError::DynamoDbPutItem)?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let now_sec = OffsetDateTime::now_utc().unix_timestamp();

        let mut attribute_names = HashMap::new();
        let mut attribute_values = HashMap::new();
        let mut key_condition = "#pk = :pk";

        let expire_av = AttributeValue::N(now_sec.to_string());
        let pk_av = AttributeValue::S(self.pk(session_id));

        attribute_names.insert("#expire_at".to_string(), self.props.expirey_name.clone());
        attribute_values.insert(":expire_at".to_string(), expire_av);

        attribute_names.insert("#pk".to_string(), self.props.partition_key.name.clone());
        attribute_values.insert(":pk".to_string(), pk_av);

        if let Some(sk) = &self.props.sort_key {
            let sk_av = AttributeValue::S(self.sk(session_id));
            attribute_names.insert("#sk".to_string(), sk.name.clone());
            attribute_values.insert(":sk".to_string(), sk_av);
            key_condition = "#pk = :pk AND #sk = :sk";
        }

        let item = self
            .client
            .query()
            .table_name(&self.props.table_name)
            .set_expression_attribute_names(Some(attribute_names))
            .set_expression_attribute_values(Some(attribute_values))
            .key_condition_expression(key_condition)
            .filter_expression("#expire_at > :expire_at")
            .send()
            .await
            .map_err(DynamoDBStoreError::DynamoDbQuery)?
            .items
            .and_then(|list| list.into_iter().next())
            .and_then(|map| {
                if let Some(AttributeValue::B(blob)) = map.get(&self.props.data_name) {
                    Some(blob.clone().into_inner())
                } else {
                    None
                }
            });

        if let Some(bytes) = item {
            Ok(Some(
                rmp_serde::from_slice(&bytes).map_err(DynamoDBStoreError::Decode)?,
            ))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        let _ = if let Some(sk) = &self.props.sort_key {
            self.client
                .delete_item()
                .table_name(&self.props.table_name)
                .key(
                    &self.props.partition_key.name,
                    AttributeValue::S(self.pk(session_id)),
                )
                .key(&sk.name, AttributeValue::S(self.sk(session_id)))
                .send()
                .await
                .map_err(DynamoDBStoreError::DynamoDbDeleteItem)?;
        } else {
            self.client
                .delete_item()
                .table_name(&self.props.table_name)
                .key(
                    &self.props.partition_key.name,
                    AttributeValue::S(self.pk(session_id)),
                )
                .send()
                .await
                .map_err(DynamoDBStoreError::DynamoDbDeleteItem)?;
        };
        Ok(())
    }
}
