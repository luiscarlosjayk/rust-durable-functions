# Rust vs Node.js: Claude Code on AWS Lambda Durable Functions

**Date:** 2026-03-20
**Region:** us-east-1
**Model:** Claude Sonnet 4.6 (via Amazon Bedrock)

## Overview

This report compares two approaches for running Claude Code as a durable Lambda function with Amazon Bedrock:

1. **Rust + CLI Layer** — A Rust binary that spawns the Claude Code CLI (`claude`) as a subprocess. The CLI is packaged as a Lambda layer.
2. **Node.js + Agent SDK** — A Node.js handler that uses the `@anthropic-ai/claude-agent-sdk` npm package directly. No layer required.

Both lambdas use AWS Lambda Durable Execution, Bedrock IAM authentication (no API keys), and were invoked with the same prompt.

## Test Configuration

| Setting | Value |
|---|---|
| Prompt | `"What is 2 + 2? Reply with just the number."` |
| Max Turns | 1 |
| Max Budget | $0.10 USD |
| Memory | 1024 MB |
| Timeout | 5 minutes |
| Durable Execution Timeout | 1 hour |
| Bedrock Model | `us.anthropic.claude-sonnet-4-6` |
| Invocation Type | Asynchronous (`Event`) |

## Lambda Functions

| | Rust (CLI Layer) | Node.js (Agent SDK) |
|---|---|---|
| **Function Name** | `rust-durable-functions-dev-claude-code-bedrock-orchestrator` | `rust-durable-functions-dev-nodejs-cc-bedrock-orchestrator` |
| **Runtime** | `nodejs:24.DurableFunction.v10` (L1 escape hatch) | `nodejs:22.DurableFunction.v10` |
| **Architecture** | ARM64 | ARM64 |
| **Approach** | Rust binary spawns `claude` CLI subprocess | `query()` from `@anthropic-ai/claude-agent-sdk` |
| **Bedrock Auth** | IAM role + env vars | IAM role + env vars |

### Architecture Differences

**Rust + CLI Layer:**
- Uses CDK's L1 escape hatch to set CloudFormation runtime to `nodejs24.x` while actually running a Rust binary via `AWS_LAMBDA_EXEC_WRAPPER: /var/task/bootstrap`
- Claude Code CLI is provided as a 33 MB Lambda layer at `/opt/nodejs/node_modules/.bin/claude`
- The Rust handler spawns a shell subprocess: `sh -c "CLAUDE_CODE_USE_BEDROCK=1 ... /opt/nodejs/node_modules/.bin/claude -p '...' --output-format json"`

**Node.js + Agent SDK:**
- Native Node.js Lambda using `NodejsFunction` CDK construct
- `@anthropic-ai/claude-agent-sdk` installed as a dependency via `commandHooks` (not esbuild-bundled, since the SDK internally spawns a `cli.js` subprocess)
- Handler uses ESM format with a `createRequire` banner shim for CommonJS compatibility
- Uses `withDurableExecution` wrapper from `@aws/durable-execution-sdk-js`

## Results

### Durable Execution Timeline

| Event | Rust (CLI Layer) | Node.js (Agent SDK) |
|---|---|---|
| **ExecutionStarted** | `00:07:27.783Z` | `00:27:00.694Z` |
| **StepStarted** | — | `00:27:02.759Z` |
| **StepSucceeded** | `00:07:33.771Z` | `00:27:07.405Z` |
| **ExecutionSucceeded** | `00:07:33.771Z` | `00:27:07.432Z` |
| **Total Wall Time** | **~6.0s** | **~6.7s** |
| **Step Duration** | ~5.2s | ~4.6s |
| **Result** | `"4"` | `{"output":"4"}` |

### Lambda Metrics (from CloudWatch REPORT)

| Metric | Rust (CLI Layer) | Node.js (Agent SDK) |
|---|---|---|
| **Init Duration (Cold Start)** | 43.6 ms | 999.3 ms |
| **Execution Duration** | 5,153 ms | 181 ms (first invocation) |
| **Billed Duration** | 5,197 ms | 1,181 ms (first), 61-96 ms (replays) |
| **Max Memory Used** | 369 MB | 115-117 MB |

### Package Size

| Metric | Rust (CLI Layer) | Node.js (Agent SDK) |
|---|---|---|
| **Function Code** | 5.44 MB | 32 MB |
| **Lambda Layer** | 33.11 MB | None |
| **Total Deployment Size** | 38.55 MB | 32 MB |

## Analysis

### Cold Start

The Rust lambda reports an artificially low Init Duration of 43.6 ms because it uses the L1 escape hatch — the durable runtime initializes the Node.js 24 environment, but the actual Rust binary and CLI layer are loaded lazily during execution. The true "ready to respond" time is init + first execution = ~5.2s.

The Node.js lambda has an honest cold start of 999 ms where it actually loads and initializes the Agent SDK. Its subsequent execution is much faster at 181 ms for the first step.

### Memory Usage

The Node.js lambda uses significantly less memory (115 MB vs 369 MB). The Rust lambda's higher memory consumption is driven by the Claude CLI subprocess and its dependencies loaded from the 33 MB layer.

### Execution Pattern

The Rust lambda completes in a single durable invocation since the CLI subprocess handles the entire agent loop internally. The Node.js lambda shows the durable execution replay pattern — multiple short invocations as the runtime checkpoints and replays through `context.step()`.

### Package Size

Both approaches end up at roughly similar total sizes (~32-38 MB), but structured differently. The Rust approach splits across a 5.4 MB binary and a 33 MB layer, while the Node.js approach is a single 32 MB deployment package containing the SDK with its bundled CLI.

### Developer Experience

The Node.js Agent SDK approach is simpler to maintain:
- No Lambda layer management
- No subprocess spawning in application code
- Standard npm dependency management
- Native TypeScript with proper types from the SDK

## Challenges Encountered

### ESM/CommonJS Compatibility

The `@anthropic-ai/claude-agent-sdk` package has a mixed module setup:
- `sdk.mjs` (main entry) is ESM
- `cli.js` (internal subprocess) is CommonJS

This required:
1. Setting esbuild output to ESM format (`format: ESM`)
2. Adding a `createRequire` banner shim for CommonJS compatibility with the durable execution SDK
3. Marking the SDK as external and installing it via `commandHooks` (not bundling with esbuild) to preserve the full package structure including `cli.js`

### The SDK Still Spawns a Subprocess

Despite being a "library", the Agent SDK internally spawns `node cli.js` as a subprocess. This means the SDK cannot be esbuild-bundled into a single file — the full `node_modules` directory structure must be preserved at runtime.

## Conclusion

Both approaches deliver comparable end-to-end performance (~6-7 seconds for a simple prompt). The Node.js Agent SDK approach offers a simpler developer experience and lower memory footprint, while the Rust approach provides a faster reported Init Duration (though this is misleading due to the escape hatch). For new projects, the Node.js Agent SDK is the recommended approach for running Claude Code on AWS Lambda with Bedrock.
