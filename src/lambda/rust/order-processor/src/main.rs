use aws_sdk_dynamodb::{Client as DynamoDBClient, types::AttributeValue};
#[allow(unused_imports)]
// Due to macro expansion, DurableContext will be marked as unused
use durable_execution_sdk::DurableContext;
use durable_execution_sdk::{DurableError, durable_execution};
use lambda_runtime::{
    Error, run, service_fn,
    tracing::{self, instrument, subscriber::EnvFilter},
};
use serde::{Deserialize, Serialize};
use shared::{Order, OrderStatus};
use std::sync::LazyLock;
use tokio::sync::OnceCell;

static DDB_CLIENT: OnceCell<DynamoDBClient> = OnceCell::const_new();
static ORDERS_TABLE: LazyLock<String> =
    LazyLock::new(|| std::env::var("ORDERS_TABLE").expect("Failed to get ORDERS_TABLE env var"));

#[derive(Debug, Deserialize)]
struct OrderEvent {
    order_id: String,
    item_name: String,
    quantity: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct CallbackResult {
    order_id: String,
}

/// Retrieves the DynamoDB client from the OnceLock, initializing it if necessary.
async fn get_dynamo_db_client() -> DynamoDBClient {
    DDB_CLIENT
        .get_or_init(|| async {
            let config = aws_config::load_from_env().await;
            DynamoDBClient::new(&config)
        })
        .await
        .clone()
}

#[durable_execution]
#[instrument(skip_all, fields(order_id = event.order_id))]
/// Handles the incoming event by interacting with DynamoDB.
async fn function_handler(
    event: OrderEvent,
    ctx: DurableContext,
) -> Result<Order, DurableError> {
    let db_client = get_dynamo_db_client().await;

    let event: OrderEvent = ctx.get_original_input()?;
    let order_id = event.order_id.clone();
    let item_name = event.item_name.clone();
    let quantity = event.quantity;

    // STEP 1: Insert into DynamoDB.
    // Wrapped in a step so it executes EXACTLY ONCE, even if the lambda replays later.
    ctx.step(
        move |_step_ctx| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    db_client
                        .put_item()
                        .table_name(ORDERS_TABLE.as_str())
                        .item("PK", AttributeValue::S(order_id.clone()))
                        .item("status", AttributeValue::S(OrderStatus::Pending.to_string()))
                        .item("item_name", AttributeValue::S(item_name))
                        .item("quantity", AttributeValue::N(quantity.to_string()))
                        .send()
                        .await
                        .map_err(|e| DurableError::execution(e.to_string()))?;

                    Ok(())
                })
            })
        },
        None,
    )
    .await?;

    // STEP 2: Create callback and store callback_id in DynamoDB
    let callback = ctx.create_callback_named::<CallbackResult>("order-approval", None).await?;
    let callback_id = callback.callback_id.clone();

    let db_client = get_dynamo_db_client().await;
    let order_id_clone = event.order_id.clone();
    ctx.step(
        move |_step_ctx| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    db_client
                        .update_item()
                        .table_name(ORDERS_TABLE.as_str())
                        .key("PK", AttributeValue::S(order_id_clone))
                        .update_expression("SET callback_id = :cid")
                        .expression_attribute_values(":cid", AttributeValue::S(callback_id))
                        .send()
                        .await
                        .map_err(|e| DurableError::execution(e.to_string()))?;
                    Ok(())
                })
            })
        },
        None,
    )
    .await?;

    // STEP 3: Wait for callback — function suspends here until callback-sender resumes it
    let callback_result: CallbackResult = callback.result().await?;

    // STEP 4: Read the updated order from DynamoDB (source of truth)
    let db_client = get_dynamo_db_client().await;
    let order_id = callback_result.order_id.clone();
    let order: Order = ctx.step(
        move |_step_ctx| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    let result = db_client
                        .get_item()
                        .table_name(ORDERS_TABLE.as_str())
                        .key("PK", AttributeValue::S(order_id.clone()))
                        .send()
                        .await
                        .map_err(|e| DurableError::execution(e.to_string()))?;

                    let item = result
                        .item()
                        .ok_or_else(|| DurableError::execution(format!("Order {order_id} not found")))?;

                    let status_str = item.get("status")
                        .and_then(|v| v.as_s().ok())
                        .ok_or_else(|| DurableError::execution("Missing status field".to_string()))?;
                    let status: OrderStatus = serde_json::from_str(&format!("\"{status_str}\""))
                        .map_err(|e| DurableError::execution(format!("Invalid status '{status_str}': {e}")))?;
                    let item_name = item.get("item_name")
                        .and_then(|v| v.as_s().ok())
                        .unwrap_or(&String::new())
                        .clone();
                    let quantity: u32 = item.get("quantity")
                        .and_then(|v| v.as_n().ok())
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0);

                    Ok(Order {
                        order_id: order_id.clone(),
                        status,
                        item_name,
                        quantity,
                    })
                })
            })
        },
        None,
    )
    .await?;

    Ok(order)
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
