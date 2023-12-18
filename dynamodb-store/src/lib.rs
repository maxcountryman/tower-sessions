use async_trait::async_trait;
pub use aws_sdk_dynamodb;
pub use aws_config;
use aws_sdk_dynamodb::{
    Client,
    primitives::{ Blob },
    types:: { WriteRequest, DeleteRequest, AttributeValue },
    operation::{
        scan::ScanError,
        batch_write_item::BatchWriteItemError,
        delete_item:: DeleteItemError,
        put_item:: PutItemError,
        query:: QueryError,
    },
};
use time::OffsetDateTime;
use tower_sessions_core::{session::Id, ExpiredDeletion, Session, SessionStore};
use std::collections::hash_map::HashMap;

/// An error type for `DynamoDBStore`.
#[derive(thiserror::Error, Debug)]
pub enum DynamoDBStoreError {
    /// A variant to map `aws_sdk_dynamodb::error::BuildError` errors.
    #[error("DynamoDb build error: {0}")]
    DynamoDbBuild(#[from] aws_sdk_dynamodb::error::BuildError),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<QueryError>` errors.
    #[error("DynamoDb query error: {0}")]
    DynamoDbQuery(#[from] aws_sdk_dynamodb::error::SdkError<QueryError>),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<PutItemError>` errors.
    #[error("DynamoDb PutItem error: {0}")]
    DynamoDbPutItem(#[from] aws_sdk_dynamodb::error::SdkError<PutItemError>),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<DeleteItemError>` errors.
    #[error("DynamoDb DeleteItem error: {0}")]
    DynamoDbDeleteItem(#[from] aws_sdk_dynamodb::error::SdkError<DeleteItemError>),
 
    /// A variant to map `aws_sdk_dynamodb::error::SdkError<BatchWriteItemError>` errors.
    #[error("DynamoDb batch write item error: {0}")]
    DynamoDbBatchWriteItem(#[from] aws_sdk_dynamodb::error::SdkError<BatchWriteItemError>),

    /// A variant to map `aws_sdk_dynamodb::error::SdkError<ScanError>` errors.
    #[error("DynamoDb scan error: {0}")]
    DynamoDbScan(#[from] aws_sdk_dynamodb::error::SdkError<ScanError>),

    /// A variant to map `rmp_serde` encode errors.
    #[error("Rust MsgPack encode error: {0}")]
    RmpSerdeEncode(#[from] rmp_serde::encode::Error),

    /// A variant to map `rmp_serde` decode errors.
    #[error("Rust MsgPack decode error: {0}")]
    RmpSerdeDecode(#[from] rmp_serde::decode::Error),
}

#[derive(Clone, Debug)]
pub struct DynamoDBStorePartitionKey {
    pub name: String,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

impl Default for DynamoDBStorePartitionKey {
    fn default() -> Self {
        DynamoDBStorePartitionKey {
            name: "session_id".to_string(),
            prefix: Some("SESSIONS::TOWER::".to_string()),
            suffix: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DynamoDBStoreSortKey {
    pub name: String,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

impl Default for DynamoDBStoreSortKey {
    fn default() -> Self {
        DynamoDBStoreSortKey {
            name: "session_id".to_string(),
            prefix: Some("SESSIONS::TOWER::".to_string()),
            suffix: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DynamoDBStoreProps {
    pub table_name: String,
    pub partition_key: DynamoDBStorePartitionKey,
    pub sort_key: Option<DynamoDBStoreSortKey>,
    pub expirey_name: String,
    pub data_name: String,
    pub session_prefix: String,
    pub session_suffix: String,
}

impl Default for DynamoDBStoreProps {
    fn default() -> Self { 
        Self {
            table_name: "tower-sessions".to_string(),
            partition_key: DynamoDBStorePartitionKey::default(),
            sort_key: None,
            expirey_name: "expire_at".to_string(),
            data_name: "data".to_string(),
            session_prefix: "SESSIONS::TOWER::".to_string(),
            session_suffix: "".to_string(),
        }
    }
}

/// A DynamoDB session store.
#[derive(Clone, Debug)]
pub struct DynamoDBStore {
    pub client: Client,
    pub props: DynamoDBStoreProps
}

impl DynamoDBStore {
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
    pub fn new(client: Client, props: DynamoDBStoreProps) -> Self {
        Self {
            client,
            props,
        }
    }

    pub fn pk<S: ToString>(&self, input: S) -> String {
        format!(
            "{}{}{}",
            self.props.partition_key.prefix.clone().unwrap_or("".to_string()),
            input.to_string(),
            self.props.partition_key.suffix.clone().unwrap_or("".to_string())
        )
    }

    pub fn sk<S: ToString>(&self, input: S) -> String {
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
    // scans are typically avoided in dynamodb, but the ExpiredDeletion trait assumes a scan is
    // available on the Store, so it is recommended to use this in conjunction with
    // a dynamo Time to Live setting on the DynamoDB table.
    // A TTL setting will let dynamodb cull expired sessions inbetween delete_expired runs,
    // preventing the scan from returning larger results, taking longer, costing more, and increasing
    // the chance for a failure while batch processing.
    // see: https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/TTL.html
    async fn delete_expired(&self) -> Result<(), Self::Error> {
        let now_sec = OffsetDateTime::now_utc().unix_timestamp();
        let now_av = AttributeValue::N(now_sec.to_string());

        // expression_attribute_names
        let mut projection = "#pk";
        let mut attribute_names = HashMap::new();
        attribute_names.insert("#expire_at".to_string(), self.props.expirey_name.clone());
        attribute_names.insert("#pk".to_string(), self.props.partition_key.name.clone());
        if let Some(sk) =  &self.props.sort_key {
            attribute_names.insert("#sk".to_string(), sk.name.clone());
            projection = "#pk, #sk";
        }

        let mut expired_sessions = self.client
            .scan()
            .table_name(&self.props.table_name)
            .set_expression_attribute_names(Some(attribute_names))
            .expression_attribute_values(":expire_at", now_av)
            .filter_expression("#expire_at < :expire_at")
            .projection_expression(projection)
            .into_paginator()
            .page_size(25)
            .items()
            .send();

        // batchwriteitem only processes 25 items at a time
        let mut batches:Vec<Vec<WriteRequest>> = Vec::with_capacity(50);
        let mut batch:Vec<WriteRequest> = Vec::with_capacity(25); 
        while let Some(session) = expired_sessions.next().await {
            if batch.len() == 25 {
                batches.push(batch);
                batch = Vec::with_capacity(25);
            }
            let delete_request = DeleteRequest::builder().set_key(Some(session?.clone())).build()?;
            let write_request = WriteRequest::builder().delete_request(delete_request).build();
            batch.push(write_request);
        }
        if batch.len() > 0 {
            batches.push(batch);
        }

        // process each batch of 25 epired sessions
        for delete_batch in batches {
            let mut unprocessed_count = delete_batch.len();
            let mut unprocessed = Some(HashMap::from([(self.props.table_name.clone(), delete_batch)]));
            while unprocessed_count > 0 {
                let new_unprocessed_items = self.client
                    .batch_write_item()
                    .set_request_items(unprocessed)
                    .send()
                    .await?
                    .unprocessed_items;
                unprocessed_count = new_unprocessed_items
                    .as_ref()
                    .map(|m| m.get(&self.props.table_name).map(|v| v.len()).unwrap_or_default())
                    .unwrap_or_default();
                unprocessed = new_unprocessed_items;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl SessionStore for DynamoDBStore {
    type Error = DynamoDBStoreError;

    async fn save(&self, session: &Session) -> Result<(), Self::Error> {
//        println!("save invoked");
        let exp_sec = session.expiry_date().unix_timestamp();
        let data_bytes = rmp_serde::to_vec(session)?;

        let mut item = HashMap::new();
        item.insert(self.props.partition_key.name.clone(), AttributeValue::S(self.pk(session.id())));
        item.insert(self.props.data_name.clone(), AttributeValue::B(Blob::new(data_bytes)));
        item.insert(self.props.expirey_name.clone(), AttributeValue::N(exp_sec.to_string()));
        if let Some(sk) = &self.props.sort_key {
            item.insert(sk.name.clone(), AttributeValue::S(self.sk(session.id())));
        }

//        println!("save item={:?}", &item);

        let mut save = self.client
            .put_item()
            .table_name(&self.props.table_name)
            .set_item(Some(item))
            .send()
            .await?;
        println!("save: {:?}", save);
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Session>, Self::Error> {
//        println!("load invoked: session_id={}", &session_id);
        let now_sec = OffsetDateTime::now_utc().unix_timestamp();

        let mut attribute_names = HashMap::new();
        let mut attribute_values = HashMap::new();
        let mut key_condition = "#pk = :pk";

        attribute_names.insert("#expire_at".to_string(), self.props.expirey_name.clone());
        attribute_values.insert(":expire_at".to_string(), AttributeValue::N(now_sec.to_string()));

        attribute_names.insert("#pk".to_string(), self.props.partition_key.name.clone());
        attribute_values.insert(":pk".to_string(), AttributeValue::S(self.pk(session_id)));

        if let Some(sk) = &self.props.sort_key {
            attribute_names.insert("#sk".to_string(), sk.name.clone());
            attribute_values.insert(":sk".to_string(), AttributeValue::S(self.sk(session_id)));
            key_condition = "#pk = :pk AND #sk = :sk";
        }

        let item = self.client
            .query()
            .table_name(&self.props.table_name)
            .set_expression_attribute_names(Some(attribute_names))
            .set_expression_attribute_values(Some(attribute_values))
            .key_condition_expression(key_condition)
            .filter_expression("#expire_at > :expire_at")
            .send()
            .await?
            .items
            .and_then(|list| list.into_iter().next())
            .and_then(|map| { 
                if let Some(AttributeValue::B(blob)) = map.get(&self.props.data_name) { Some(blob.clone().into_inner()) }
                else { None }
            });

//        println!("load: {:?}", &item);

        if let Some(bytes) = item {
            Ok(Some(rmp_serde::from_slice(&bytes)?))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &Id) -> Result<(), Self::Error> {
//        println!("delete invoked");
        let _ = if let Some(sk) = &self.props.sort_key {
            self.client
                .delete_item()
                .table_name(&self.props.table_name)
                .key(&self.props.partition_key.name, AttributeValue::S(self.pk(session_id)))
                .key(&sk.name, AttributeValue::S(self.sk(session_id)))
                .send()
                .await?;
        } else {
            self.client
                .delete_item()
                .table_name(&self.props.table_name)
                .key(&self.props.partition_key.name, AttributeValue::S(self.pk(session_id)))
                .send()
                .await?;
        };
        Ok(())
    }
}

