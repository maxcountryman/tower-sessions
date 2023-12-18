use std::net::SocketAddr;
use axum::{
    error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use time::Duration;
use tower::ServiceBuilder;
use tower_sessions::{
    aws_config,
    aws_sdk_dynamodb,
    Expiry, DynamoDBStore, DynamoDBStoreProps, DynamoDBStoreSortKey, Session, SessionManagerLayer
};
const COUNTER_KEY: &str = "counter";

#[derive(Serialize, Deserialize, Default)]
struct Counter(usize);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // credentials are unused, but nessassary when configuring aws_config
    // see: https://docs.aws.amazon.com/sdk-for-rust/latest/dg/dynamodb-local.html
    std::env::set_var("AWS_REGION", "us-east-1");
    std::env::set_var("AWS_ACCESS_KEY_ID", "AKIDLOCALSTACK");
    std::env::set_var("AWS_SECRET_ACCESS_KEY", "localstacksecret");
    
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region("us-east-1")
        .load()
        .await;
    
    let dynamodb_local_config = aws_sdk_dynamodb::config::Builder::from(&config)
        .endpoint_url("http://localhost:8000") // 8000 is the default dynamodb port, check test/docker-compose.yml
        .build();

    let client = aws_sdk_dynamodb::Client::from_conf(dynamodb_local_config);

    let store_props = DynamoDBStoreProps { 
        table_name: "TowerSessions".to_string(),
        sort_key: Some(DynamoDBStoreSortKey {
            name: "sort_key".to_string(),
            prefix: Some("TOWER_SESSIONS::".to_string()),
            suffix: None,
        }),
        ..Default::default()
    };

    let mut create_table_request = client.create_table()
        .table_name(&store_props.table_name)
        .attribute_definitions(aws_sdk_dynamodb::types::AttributeDefinition::builder()
            .attribute_name(&store_props.partition_key.name)
            .attribute_type(aws_sdk_dynamodb::types::ScalarAttributeType::S)
            .build()
            .unwrap())
        .key_schema(aws_sdk_dynamodb::types::KeySchemaElement::builder()
            .attribute_name(&store_props.partition_key.name)
            .key_type(aws_sdk_dynamodb::types::KeyType::Hash)
            .build()
            .unwrap())
        .provisioned_throughput(aws_sdk_dynamodb::types::ProvisionedThroughput::builder()
            .read_capacity_units(10)
            .write_capacity_units(5)
            .build()
            .unwrap());

    if let Some(sk) = &store_props.sort_key {
        create_table_request = create_table_request
            .attribute_definitions(aws_sdk_dynamodb::types::AttributeDefinition::builder()
                    .attribute_name(&sk.name)
                    .attribute_type(aws_sdk_dynamodb::types::ScalarAttributeType::S)
                    .build()
                    .unwrap())
            .key_schema(
                aws_sdk_dynamodb::types::KeySchemaElement::builder()
                    .attribute_name(&sk.name)
                    .key_type(aws_sdk_dynamodb::types::KeyType::Range)
                    .build()
                    .unwrap());
    }

    let _create_table_response = create_table_request.send().await;


    let session_store = DynamoDBStore::new(client, store_props);

    let session_service = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|e: BoxError| async move {
            println!("error: {:?}",e);
            StatusCode::BAD_REQUEST
        }))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_expiry(Expiry::OnInactivity(Duration::seconds(10))),
        );

    let app = Router::new()
        .route("/", get(handler))
        .layer(session_service);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn handler(session: Session) -> impl IntoResponse {
    let counter: Counter = session
        .get(COUNTER_KEY)
        .expect("Could not deserialize.")
        .unwrap_or_default();

    session
        .insert(COUNTER_KEY, counter.0 + 1)
        .expect("Could not serialize.");

    format!("Current count: {}", counter.0)
}
