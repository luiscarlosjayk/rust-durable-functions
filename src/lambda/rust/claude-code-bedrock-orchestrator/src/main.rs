#[allow(unused_imports)]
use durable_execution_sdk::DurableContext;
use durable_execution_sdk::{DurableError, durable_execution};
use lambda_runtime::{
    Error, run, service_fn,
    tracing::{self, subscriber::EnvFilter},
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::info;

fn default_max_turns() -> u32 {
    10
}

fn default_max_budget() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
struct ClaudeCodeRequest {
    prompt: String,
    #[serde(default = "default_max_turns")]
    max_turns: u32,
    #[serde(default = "default_max_budget")]
    max_budget_usd: f64,
    /// Optional: working directory (e.g., cloned repo in /tmp)
    working_dir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClaudeCodeResult {
    output: serde_json::Value,
}

#[durable_execution]
async fn function_handler(
    _event: ClaudeCodeRequest,
    ctx: DurableContext,
) -> Result<ClaudeCodeResult, DurableError> {
    let event: ClaudeCodeRequest = ctx.get_original_input()?;

    info!(
        prompt = %event.prompt,
        max_turns = event.max_turns,
        max_budget_usd = event.max_budget_usd,
        "Starting Claude Code CLI execution (Bedrock)"
    );

    let prompt = event.prompt.clone();
    let max_turns = event.max_turns;
    let max_budget = event.max_budget_usd;
    let working_dir = event.working_dir.clone();

    let output: ClaudeCodeResult = ctx
        .step(
            move |_| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let claude_bin = "/opt/nodejs/node_modules/.bin/claude";

                        info!("Spawning claude CLI subprocess (Bedrock)");

                        let shell_cmd = format!(
                            "CLAUDE_CODE_USE_BEDROCK=1 \
                             ANTHROPIC_DEFAULT_SONNET_MODEL=global.anthropic.claude-sonnet-4-6 \
                             ANTHROPIC_DEFAULT_HAIKU_MODEL=global.anthropic.claude-sonnet-4-6 \
                             ANTHROPIC_DEFAULT_OPUS_MODEL=global.anthropic.claude-sonnet-4-6 \
                             HOME=/tmp \
                             {} -p '{}' --output-format json --max-turns {} --max-budget-usd {} 2>&1",
                            claude_bin,
                            prompt.replace('\'', "'\\''"),
                            max_turns,
                            max_budget,
                        );

                        let mut cmd = Command::new("sh");
                        cmd.arg("-c").arg(&shell_cmd);

                        if let Some(dir) = &working_dir {
                            cmd.current_dir(dir);
                        }

                        let output = cmd.output().await
                            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                format!("Failed to spawn claude CLI: {e}").into()
                            })?;

                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);

                        info!(
                            exit_code = ?output.status.code(),
                            stdout = %stdout,
                            stderr = %stderr,
                            "Claude CLI finished"
                        );

                        let parsed: serde_json::Value =
                            serde_json::from_str(&stdout).unwrap_or_else(|_| {
                                serde_json::json!({
                                    "raw_output": stdout.to_string(),
                                    "stderr": stderr.to_string(),
                                    "exit_code": output.status.code(),
                                })
                            });

                        Ok(ClaudeCodeResult { output: parsed })
                    })
                })
            },
            None,
        )
        .await?;

    info!(result = %output.output, "Claude Code execution complete");

    Ok(output)
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
