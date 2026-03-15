use aws_sdk_dynamodb::{Client as DynamoDBClient, types::AttributeValue};
use aws_sdk_lambda::{Client as LambdaClient, primitives::Blob};
use lambda_runtime::{
    Error, LambdaEvent, run, service_fn,
    tracing::{self, instrument, subscriber::EnvFilter},
};
use serde::{Deserialize, Serialize};
use shared::{Order, OrderStatus};
use std::sync::LazyLock;

static ORDERS_TABLE: LazyLock<String> =
    LazyLock::new(|| std::env::var("ORDERS_TABLE").expect("ORDERS_TABLE not set"));

#[derive(Debug, Deserialize)]
struct FetchRequest {
    callback_id: String,
    #[allow(dead_code)]
    orchestrator_function_name: String,
}

#[derive(Debug, Serialize)]
struct OrdersCallbackPayload {
    orders_json: String,
}

fn parse_order(item: &std::collections::HashMap<String, AttributeValue>) -> Result<Order, String> {
    let order_id = item
        .get("PK")
        .and_then(|v| v.as_s().ok())
        .ok_or("Missing PK")?
        .clone();

    let status_str = item
        .get("status")
        .and_then(|v| v.as_s().ok())
        .ok_or("Missing status")?;

    let status: OrderStatus = serde_json::from_str(&format!("\"{status_str}\""))
        .map_err(|e| format!("Invalid status: {e}"))?;

    let item_name = item
        .get("item_name")
        .and_then(|v| v.as_s().ok())
        .unwrap_or(&String::new())
        .clone();

    let quantity: u32 = item
        .get("quantity")
        .and_then(|v| v.as_n().ok())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);

    Ok(Order {
        order_id,
        status,
        item_name,
        quantity,
    })
}

#[instrument(skip_all, fields(callback_id = %event.payload.callback_id, orchestrator = %event.payload.orchestrator_function_name))]
async fn function_handler(event: LambdaEvent<FetchRequest>) -> Result<(), Error> {
    tracing::info!("Orders-fetcher invoked");
    let request = event.payload;

    let config = aws_config::load_from_env().await;
    let ddb_client = DynamoDBClient::new(&config);
    let lambda_client = LambdaClient::new(&config);

    // Scan DynamoDB for all orders
    let scan_result = ddb_client
        .scan()
        .table_name(ORDERS_TABLE.as_str())
        .send()
        .await
        .map_err(|e| Error::from(format!("DynamoDB scan failed: {e}")))?;

    let orders: Vec<Order> = scan_result
        .items()
        .iter()
        .filter_map(|item| match parse_order(item) {
            Ok(order) => Some(order),
            Err(e) => {
                tracing::warn!(error = %e, "Skipping malformed order item");
                None
            }
        })
        .collect();

    tracing::info!(order_count = orders.len(), "Fetched orders from DynamoDB");

    // Send callback with orders data
    let orders_json = serde_json::to_string(&orders)?;
    tracing::info!(orders_json_len = orders_json.len(), "Sending orders callback");
    let payload = OrdersCallbackPayload { orders_json };

    lambda_client
        .send_durable_execution_callback_success()
        .callback_id(&request.callback_id)
        .result(Blob::new(serde_json::to_vec(&payload)?))
        .send()
        .await
        .map_err(|e| Error::from(format!("Failed to send callback: {e:?}")))?;

    tracing::info!("Callback sent successfully");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing::subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .with_current_span(false)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .init();

    run(service_fn(function_handler)).await
}
