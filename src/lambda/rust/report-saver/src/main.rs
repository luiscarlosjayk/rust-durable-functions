use aws_sdk_lambda::{Client as LambdaClient, primitives::Blob};
use aws_sdk_s3::Client as S3Client;
use lambda_runtime::{
    Error, LambdaEvent, run, service_fn,
    tracing::{self, instrument, subscriber::EnvFilter},
};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use uuid::Uuid;

static REPORTS_BUCKET: LazyLock<String> =
    LazyLock::new(|| std::env::var("REPORTS_BUCKET").expect("REPORTS_BUCKET not set"));

#[derive(Debug, Deserialize)]
struct SaveRequest {
    callback_id: String,
    #[allow(dead_code)]
    orchestrator_function_name: String,
    title: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct SaveCallbackPayload {
    report_url: String,
}

#[instrument(skip_all, fields(callback_id = %event.payload.callback_id, title = %event.payload.title, content_len = event.payload.content.len()))]
async fn function_handler(event: LambdaEvent<SaveRequest>) -> Result<(), Error> {
    tracing::info!("Report-saver invoked");
    let request = event.payload;

    let config = aws_config::load_from_env().await;
    let s3_client = S3Client::new(&config);
    let lambda_client = LambdaClient::new(&config);

    let bucket = REPORTS_BUCKET.as_str();

    // Generate S3 key and write markdown
    let key = format!("reports/{}.md", Uuid::new_v4());

    s3_client
        .put_object()
        .bucket(bucket)
        .key(&key)
        .body(request.content.into_bytes().into())
        .content_type("text/markdown")
        .send()
        .await
        .map_err(|e| Error::from(format!("S3 put_object failed: {e}")))?;

    let report_url = format!("s3://{bucket}/{key}");
    tracing::info!(report_url = %report_url, "Report saved to S3");

    // Send callback with report URL
    let payload = SaveCallbackPayload { report_url };

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
