use aws_sdk_bedrockruntime::{
    Client as BedrockClient,
    types::{
        ContentBlock, ConversationRole, Message, SystemContentBlock,
        Tool, ToolConfiguration, ToolInputSchema, ToolResultBlock,
        ToolResultContentBlock, ToolSpecification, ToolUseBlock,
    },
};
use aws_sdk_lambda::{Client as LambdaClient, primitives::Blob, types::InvocationType};
use aws_sdk_sns::Client as SnsClient;
use aws_smithy_types::{Document, Number as SmithyNumber};
#[allow(unused_imports)]
use durable_execution_sdk::DurableContext;
use durable_execution_sdk::{DurableError, durable_execution};
use lambda_runtime::{
    Error, run, service_fn,
    tracing::{self, instrument, subscriber::EnvFilter},
};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use tokio::sync::OnceCell;

static BEDROCK_CLIENT: OnceCell<BedrockClient> = OnceCell::const_new();
static LAMBDA_CLIENT: OnceCell<LambdaClient> = OnceCell::const_new();
static SNS_CLIENT: OnceCell<SnsClient> = OnceCell::const_new();

static MODEL_ID: LazyLock<String> =
    LazyLock::new(|| std::env::var("BEDROCK_MODEL_ID").expect("BEDROCK_MODEL_ID not set"));
static ORDERS_FETCHER_FN: LazyLock<String> = LazyLock::new(|| {
    std::env::var("ORDERS_FETCHER_FUNCTION_NAME").expect("ORDERS_FETCHER_FUNCTION_NAME not set")
});
static REPORT_SAVER_FN: LazyLock<String> = LazyLock::new(|| {
    std::env::var("REPORT_SAVER_FUNCTION_NAME").expect("REPORT_SAVER_FUNCTION_NAME not set")
});
static NOTIFICATIONS_TOPIC_ARN: LazyLock<String> =
    LazyLock::new(|| std::env::var("NOTIFICATIONS_TOPIC_ARN").expect("NOTIFICATIONS_TOPIC_ARN not set"));

async fn get_bedrock_client() -> BedrockClient {
    BEDROCK_CLIENT
        .get_or_init(|| async {
            let config = aws_config::load_from_env().await;
            BedrockClient::new(&config)
        })
        .await
        .clone()
}

async fn get_lambda_client() -> LambdaClient {
    LAMBDA_CLIENT
        .get_or_init(|| async {
            let config = aws_config::load_from_env().await;
            LambdaClient::new(&config)
        })
        .await
        .clone()
}

async fn get_sns_client() -> SnsClient {
    SNS_CLIENT
        .get_or_init(|| async {
            let config = aws_config::load_from_env().await;
            SnsClient::new(&config)
        })
        .await
        .clone()
}

// --- Request/Response types ---

#[derive(Debug, Deserialize)]
struct ReportRequest {
    report_topic: String,
}

#[derive(Debug, Serialize)]
struct ReportResult {
    report_url: String,
    notification_sent: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct OrdersCallbackResult {
    orders_json: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct SaveReportCallbackResult {
    report_url: String,
}

#[derive(Debug, Serialize)]
struct FetchRequest {
    callback_id: String,
    orchestrator_function_name: String,
}

#[derive(Debug, Serialize)]
struct SaveRequest {
    callback_id: String,
    orchestrator_function_name: String,
    title: String,
    content: String,
}

// --- Serializable types for durable step results ---
// ConverseOutput doesn't implement Serialize/Deserialize, so we extract
// the parts we need into these types for durable step caching.

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConverseStepOutput {
    stop_reason: String,
    content_blocks: Vec<SerializableBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum SerializableBlock {
    Text { text: String },
    ToolUse { tool_use_id: String, name: String, input_json: String },
}

// --- Helpers ---

/// Convert serde_json::Value to aws_smithy_types::Document
fn json_to_document(value: &serde_json::Value) -> Document {
    match value {
        serde_json::Value::Null => Document::Null,
        serde_json::Value::Bool(b) => Document::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Document::Number(SmithyNumber::PosInt(i as u64))
                } else {
                    Document::Number(SmithyNumber::NegInt(i))
                }
            } else if let Some(f) = n.as_f64() {
                Document::Number(SmithyNumber::Float(f))
            } else {
                Document::Null
            }
        }
        serde_json::Value::String(s) => Document::String(s.clone()),
        serde_json::Value::Array(arr) => {
            Document::Array(arr.iter().map(json_to_document).collect())
        }
        serde_json::Value::Object(map) => {
            Document::Object(map.iter().map(|(k, v)| (k.clone(), json_to_document(v))).collect())
        }
    }
}

/// Convert aws_smithy_types::Document to serde_json::Value
fn document_to_json(doc: &Document) -> serde_json::Value {
    match doc {
        Document::Null => serde_json::Value::Null,
        Document::Bool(b) => serde_json::Value::Bool(*b),
        Document::Number(n) => match n {
            SmithyNumber::PosInt(i) => serde_json::json!(*i),
            SmithyNumber::NegInt(i) => serde_json::json!(*i),
            SmithyNumber::Float(f) => serde_json::json!(*f),
        },
        Document::String(s) => serde_json::Value::String(s.clone()),
        Document::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(document_to_json).collect())
        }
        Document::Object(map) => {
            serde_json::Value::Object(
                map.iter()
                    .map(|(k, v): (&String, &Document)| (k.clone(), document_to_json(v)))
                    .collect(),
            )
        }
    }
}

fn build_tool_config() -> ToolConfiguration {
    let get_orders_schema = json_to_document(&serde_json::json!({
        "type": "object",
        "properties": {},
    }));

    let save_report_schema = json_to_document(&serde_json::json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "description": "Report title"
            },
            "content": {
                "type": "string",
                "description": "Full markdown content of the report"
            }
        },
        "required": ["title", "content"]
    }));

    ToolConfiguration::builder()
        .tools(Tool::ToolSpec(
            ToolSpecification::builder()
                .name("get_orders_statuses")
                .description("Retrieves all order statuses from the database. Returns a JSON array of orders with order_id, status, item_name, and quantity.")
                .input_schema(ToolInputSchema::Json(get_orders_schema))
                .build()
                .expect("valid tool spec"),
        ))
        .tools(Tool::ToolSpec(
            ToolSpecification::builder()
                .name("save_report")
                .description("Saves a markdown report to persistent storage. Returns the URL where the report was saved.")
                .input_schema(ToolInputSchema::Json(save_report_schema))
                .build()
                .expect("valid tool spec"),
        ))
        .build()
        .expect("valid tool config")
}

fn get_self_function_name() -> String {
    std::env::var("AWS_LAMBDA_FUNCTION_NAME").unwrap_or_else(|_| "report-orchestrator".to_string())
}

/// Reconstruct a Bedrock Message from our serializable blocks (for conversation history)
fn reconstruct_assistant_message(output: &ConverseStepOutput) -> Result<Message, DurableError> {
    let content_blocks: Vec<ContentBlock> = output
        .content_blocks
        .iter()
        .map(|b| match b {
            SerializableBlock::Text { text } => ContentBlock::Text(text.clone()),
            SerializableBlock::ToolUse { tool_use_id, name, input_json } => {
                let input_doc = json_to_document(
                    &serde_json::from_str(input_json).unwrap_or_else(|e| {
                        tracing::warn!(error = %e, tool_use_id, name, "Failed to parse tool input JSON in reconstruct_assistant_message");
                        serde_json::Value::Object(Default::default())
                    }),
                );
                ContentBlock::ToolUse(
                    ToolUseBlock::builder()
                        .tool_use_id(tool_use_id)
                        .name(name)
                        .input(input_doc)
                        .build()
                        .expect("valid tool use block"),
                )
            }
        })
        .collect();

    Message::builder()
        .role(ConversationRole::Assistant)
        .set_content(Some(content_blocks))
        .build()
        .map_err(|e| DurableError::execution(e.to_string()))
}

#[durable_execution]
#[instrument(skip_all, fields(report_topic))]
async fn function_handler(
    _event: ReportRequest,
    ctx: DurableContext,
) -> Result<ReportResult, DurableError> {
    let event: ReportRequest = ctx.get_original_input().map_err(|e| {
        tracing::error!(error = %e, "Failed to get original input");
        e
    })?;
    tracing::Span::current().record("report_topic", event.report_topic.as_str());
    tracing::info!(report_topic = %event.report_topic, "Starting report orchestration");

    let system_prompt = SystemContentBlock::Text(
        "You are a report generator. Use get_orders_statuses tool to fetch current order data, \
         then generate a comprehensive markdown report and save it using save_report tool."
            .to_string(),
    );

    let tools = build_tool_config();

    let mut messages: Vec<Message> = vec![
        Message::builder()
            .role(ConversationRole::User)
            .content(ContentBlock::Text(event.report_topic.clone()))
            .build()
            .map_err(|e| DurableError::execution(e.to_string()))?,
    ];

    let mut report_url = String::new();
    let mut iteration = 0u32;

    // Tool loop: call Bedrock Converse API until model returns end_turn
    loop {
        iteration += 1;
        tracing::info!(iteration, "Starting Bedrock Converse iteration");
        let bedrock_client = get_bedrock_client().await;
        let model_id = MODEL_ID.clone();
        let system_prompt_clone = system_prompt.clone();
        let tools_clone = tools.clone();
        let messages_clone = messages.clone();

        // Step: Call Bedrock Converse API
        // Returns a serializable struct since ConverseOutput doesn't impl Serialize
        let step_output: ConverseStepOutput = ctx
            .step(
                move |_| {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            let response = bedrock_client
                                .converse()
                                .model_id(model_id)
                                .system(system_prompt_clone)
                                .set_messages(Some(messages_clone))
                                .tool_config(tools_clone)
                                .send()
                                .await
                                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                    tracing::error!(error = ?e, "Bedrock Converse API call failed");
                                    format!("{e:?}").into()
                                })?;

                            let stop_reason = format!("{:?}", response.stop_reason());

                            let converse_output = response
                                .output()
                                .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                                    tracing::error!("No output in Converse response");
                                    "No output in Converse response".into()
                                })?;

                            let output_msg = converse_output
                                .as_message()
                                .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                                    tracing::error!("Converse output is not a message");
                                    "Output is not a message".into()
                                })?;

                            let content_blocks: Vec<SerializableBlock> = output_msg
                                .content()
                                .iter()
                                .filter_map(|block| match block {
                                    ContentBlock::Text(text) => {
                                        Some(SerializableBlock::Text { text: text.clone() })
                                    }
                                    ContentBlock::ToolUse(tu) => {
                                        let input_json = serde_json::to_string(
                                            &document_to_json(tu.input()),
                                        )
                                        .unwrap_or_else(|e| {
                                            tracing::warn!(error = %e, tool_name = %tu.name(), "Failed to serialize tool input to JSON");
                                            String::default()
                                        });
                                        Some(SerializableBlock::ToolUse {
                                            tool_use_id: tu.tool_use_id().to_string(),
                                            name: tu.name().to_string(),
                                            input_json,
                                        })
                                    }
                                    _ => None,
                                })
                                .collect();

                            Ok(ConverseStepOutput {
                                stop_reason,
                                content_blocks,
                            })
                        })
                    })
                },
                None,
            )
            .await?;

        tracing::info!(
            iteration,
            stop_reason = %step_output.stop_reason,
            content_blocks = step_output.content_blocks.len(),
            "Converse step completed"
        );

        // Reconstruct the assistant message for conversation history
        let assistant_message = reconstruct_assistant_message(&step_output)?;
        messages.push(assistant_message);

        match step_output.stop_reason.as_str() {
            "EndTurn" => break,
            "ToolUse" => {
                let mut tool_result_blocks: Vec<ContentBlock> = vec![];

                for block in &step_output.content_blocks {
                    if let SerializableBlock::ToolUse { tool_use_id, name, input_json } = block {
                        match name.as_str() {
                            "get_orders_statuses" => {
                                // Create callback for orders-fetcher
                                let callback = ctx
                                    .create_callback_named::<OrdersCallbackResult>(
                                        "orders-fetch",
                                        None,
                                    )
                                    .await?;
                                tracing::info!(
                                    callback_id = %callback.callback_id,
                                    callback_name = "orders-fetch",
                                    "Created callback for orders-fetcher"
                                );

                                // Step: Async invoke orders-fetcher
                                let lambda_client = get_lambda_client().await;
                                let cb_id = callback.callback_id.clone();
                                let self_fn = get_self_function_name();
                                let fetcher_fn = ORDERS_FETCHER_FN.clone();
                                tracing::info!(function_name = %fetcher_fn, "Invoking orders-fetcher Lambda");
                                ctx.step(
                                    move |_| {
                                        tokio::task::block_in_place(|| {
                                            tokio::runtime::Handle::current().block_on(async {
                                                let payload =
                                                    serde_json::to_vec(&FetchRequest {
                                                        callback_id: cb_id,
                                                        orchestrator_function_name: self_fn,
                                                    })
                                                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                                        e.to_string().into()
                                                    })?;

                                                lambda_client
                                                    .invoke()
                                                    .function_name(fetcher_fn)
                                                    .payload(Blob::new(payload))
                                                    .invocation_type(InvocationType::Event)
                                                    .send()
                                                    .await
                                                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                                        tracing::error!(error = %e, "Failed to invoke orders-fetcher Lambda");
                                                        e.to_string().into()
                                                    })?;

                                                Ok(())
                                            })
                                        })
                                    },
                                    None,
                                )
                                .await?;

                                // Suspend until orders-fetcher sends callback
                                let result: OrdersCallbackResult =
                                    callback.result().await.map_err(|e| {
                                        tracing::error!(error = %e, "orders-fetch callback failed");
                                        e
                                    })?;
                                tracing::info!(
                                    orders_json_len = result.orders_json.len(),
                                    "Resumed from orders-fetch callback"
                                );

                                tool_result_blocks.push(ContentBlock::ToolResult(
                                    ToolResultBlock::builder()
                                        .tool_use_id(tool_use_id)
                                        .content(ToolResultContentBlock::Text(
                                            result.orders_json,
                                        ))
                                        .build()
                                        .map_err(|e| {
                                            tracing::error!(error = %e, "Failed to build ToolResultBlock for orders");
                                            DurableError::execution(e.to_string())
                                        })?,
                                ));
                            }
                            "save_report" => {
                                // Parse tool input
                                let input: serde_json::Value =
                                    serde_json::from_str(input_json)
                                        .unwrap_or_else(|e| {
                                            tracing::warn!(error = %e, input_json, "Failed to parse save_report input JSON, using default");
                                            serde_json::Value::default()
                                        });
                                let title = input
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_else(|| {
                                        tracing::warn!("Missing 'title' in save_report input, defaulting to 'report'");
                                        "report"
                                    })
                                    .to_string();
                                let content = input
                                    .get("content")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_else(|| {
                                        tracing::warn!("Missing 'content' in save_report input, defaulting to empty string");
                                        ""
                                    })
                                    .to_string();

                                // Create callback for report-saver
                                let callback = ctx
                                    .create_callback_named::<SaveReportCallbackResult>(
                                        "report-save",
                                        None,
                                    )
                                    .await?;
                                tracing::info!(
                                    callback_id = %callback.callback_id,
                                    callback_name = "report-save",
                                    title = %title,
                                    content_len = content.len(),
                                    "Created callback for report-saver"
                                );

                                // Step: Async invoke report-saver
                                let lambda_client = get_lambda_client().await;
                                let cb_id = callback.callback_id.clone();
                                let self_fn = get_self_function_name();
                                let saver_fn = REPORT_SAVER_FN.clone();
                                tracing::info!(function_name = %saver_fn, "Invoking report-saver Lambda");
                                let title_clone = title.clone();
                                let content_clone = content.clone();
                                ctx.step(
                                    move |_| {
                                        tokio::task::block_in_place(|| {
                                            tokio::runtime::Handle::current().block_on(async {
                                                let payload =
                                                    serde_json::to_vec(&SaveRequest {
                                                        callback_id: cb_id,
                                                        orchestrator_function_name: self_fn,
                                                        title: title_clone,
                                                        content: content_clone,
                                                    })
                                                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                                        e.to_string().into()
                                                    })?;

                                                lambda_client
                                                    .invoke()
                                                    .function_name(saver_fn)
                                                    .payload(Blob::new(payload))
                                                    .invocation_type(InvocationType::Event)
                                                    .send()
                                                    .await
                                                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                                        tracing::error!(error = %e, "Failed to invoke report-saver Lambda");
                                                        e.to_string().into()
                                                    })?;

                                                Ok(())
                                            })
                                        })
                                    },
                                    None,
                                )
                                .await?;

                                // Suspend until report-saver sends callback
                                let result: SaveReportCallbackResult =
                                    callback.result().await.map_err(|e| {
                                        tracing::error!(error = %e, "report-save callback failed");
                                        e
                                    })?;
                                report_url = result.report_url.clone();
                                tracing::info!(
                                    report_url = %report_url,
                                    "Resumed from report-save callback"
                                );

                                let result_json = serde_json::to_string(&result)
                                    .unwrap_or_else(|e| {
                                        tracing::warn!(error = %e, "Failed to serialize SaveReportCallbackResult");
                                        String::default()
                                    });
                                tool_result_blocks.push(ContentBlock::ToolResult(
                                    ToolResultBlock::builder()
                                        .tool_use_id(tool_use_id)
                                        .content(ToolResultContentBlock::Text(result_json))
                                        .build()
                                        .map_err(|e| {
                                            tracing::error!(error = %e, "Failed to build ToolResultBlock for save_report");
                                            DurableError::execution(e.to_string())
                                        })?,
                                ));
                            }
                            other => {
                                tracing::warn!("Unknown tool: {other}");
                            }
                        }
                    }
                }

                // Add tool results as user message for next Converse turn
                messages.push(
                    Message::builder()
                        .role(ConversationRole::User)
                        .set_content(Some(tool_result_blocks))
                        .build()
                        .map_err(|e| DurableError::execution(e.to_string()))?,
                );
            }
            other => {
                tracing::warn!("Unexpected stop reason: {other}");
                break;
            }
        }
    }

    // Step: Publish SNS notification
    tracing::info!(topic_arn = %*NOTIFICATIONS_TOPIC_ARN, report_url = %report_url, "Publishing SNS notification");
    let sns_client = get_sns_client().await;
    let topic_arn = NOTIFICATIONS_TOPIC_ARN.clone();
    let url = report_url.clone();
    ctx.step(
        move |_| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    sns_client
                        .publish()
                        .topic_arn(&topic_arn)
                        .subject("Report Generated")
                        .message(format!("Report saved at: {url}"))
                        .send()
                        .await
                        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                            tracing::error!(error = %e, "SNS publish failed");
                            e.to_string().into()
                        })?;

                    Ok(())
                })
            })
        },
        None,
    )
    .await?;

    tracing::info!(report_url = %report_url, "Report orchestration completed successfully");
    Ok(ReportResult {
        report_url,
        notification_sent: true,
    })
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
