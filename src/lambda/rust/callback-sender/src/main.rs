use aws_sdk_dynamodb::{Client as DynamoDBClient, types::AttributeValue};
use aws_sdk_lambda::Client as LambdaClient;
use aws_sdk_lambda::primitives::Blob;
use lambda_http::{
    Body, Error, Request, RequestExt, Response,
    http::{Method, StatusCode},
    run, service_fn,
    tracing::{self, instrument, subscriber::EnvFilter},
};
use serde::{Deserialize, Serialize};
use shared::OrderStatus;

#[derive(Deserialize)]
struct CallbackRequest {
    order_id: String,
    status: OrderStatus,
}

#[derive(Serialize)]
struct CallbackPayload {
    order_id: String,
}

#[instrument(skip(event), fields(method = %event.method(), uri = %event.uri(), order_id))]
async fn function_handler(event: Request) -> Result<Response<Body>, Error> {
    let body: CallbackRequest = match *event.method() {
        Method::GET => {
            let params = event.query_string_parameters();
            let order_id = params
                .first("order_id")
                .ok_or_else(|| Error::from("Missing query param: order_id"))?
                .to_string();
            let status_str = params
                .first("status")
                .ok_or_else(|| Error::from("Missing query param: status"))?;
            let status: OrderStatus =
                serde_json::from_value(serde_json::Value::String(status_str.to_string()))
                    .map_err(|e| Error::from(format!("Invalid status value: {e}")))?;
            CallbackRequest { order_id, status }
        }
        Method::POST => serde_json::from_slice(event.body().as_ref())?,
        _ => {
            return Ok(Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"error":"Method not allowed"}"#))?);
        }
    };

    // Record order_id in the current tracing span
    tracing::Span::current().record("order_id", &body.order_id.as_str());

    let config = aws_config::load_from_env().await;
    let lambda_client = LambdaClient::new(&config);
    let ddb_client = DynamoDBClient::new(&config);

    let table_name =
        std::env::var("ORDERS_TABLE").map_err(|_| Error::from("ORDERS_TABLE env var not set"))?;

    // Read order from DynamoDB to get callback_id
    let get_result = ddb_client
        .get_item()
        .table_name(&table_name)
        .key("PK", AttributeValue::S(body.order_id.clone()))
        .send()
        .await
        .map_err(|e| Error::from(format!("Failed to get order from DynamoDB: {e}")))?;

    let item = get_result
        .item()
        .ok_or_else(|| Error::from(format!("Order {} not found in DynamoDB", body.order_id)))?;

    let callback_id = item
        .get("callback_id")
        .and_then(|v| v.as_s().ok())
        .ok_or_else(|| Error::from(format!("No callback_id found for order {}", body.order_id)))?;

    // Update order status in DynamoDB
    ddb_client
        .update_item()
        .table_name(&table_name)
        .key("PK", AttributeValue::S(body.order_id.clone()))
        .update_expression("SET #s = :status")
        .expression_attribute_names("#s", "status")
        .expression_attribute_values(":status", AttributeValue::S(body.status.to_string()))
        .send()
        .await
        .map_err(|e| Error::from(format!("Failed to update order status: {e}")))?;

    // Send callback with just order_id
    let callback_payload = CallbackPayload {
        order_id: body.order_id.clone(),
    };

    lambda_client
        .send_durable_execution_callback_success()
        .callback_id(callback_id)
        .result(Blob::new(serde_json::to_vec(&callback_payload)?))
        .send()
        .await
        .map_err(|e| Error::from(format!("Failed to send callback: {e:?}")))?;

    let response_body = serde_json::json!({
        "message": format!("Callback sent for order {}", body.order_id),
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response_body)?))?)
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
