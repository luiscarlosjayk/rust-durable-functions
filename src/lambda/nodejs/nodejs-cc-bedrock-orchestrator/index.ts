import {
    withDurableExecution,
    type DurableContext,
} from "@aws/durable-execution-sdk-js";
import { query } from "@anthropic-ai/claude-agent-sdk";

interface ClaudeAgentRequest {
    prompt: string;
    maxTurns?: number;
    maxBudgetUsd?: number;
}

interface ClaudeAgentResult {
    output: unknown;
}

export const handler = withDurableExecution(
    async (event: ClaudeAgentRequest, context: DurableContext) => {
        const { prompt, maxTurns = 10, maxBudgetUsd = 1.0 } = event;

        console.log(
            JSON.stringify({
                message: "Starting Claude Agent SDK execution (Bedrock)",
                prompt,
                maxTurns,
                maxBudgetUsd,
            }),
        );

        const result: ClaudeAgentResult = await context.step(async () => {
            let finalResult: unknown = null;

            for await (const message of query({
                prompt,
                options: {
                    maxTurns,
                    maxBudgetUsd,
                    permissionMode: "bypassPermissions",
                    allowDangerouslySkipPermissions: true,
                    cwd: "/tmp",
                    env: {
                        ...(process.env as Record<string, string>),
                        HOME: "/tmp",
                        CLAUDE_CODE_USE_BEDROCK: "1",
                        ANTHROPIC_DEFAULT_SONNET_MODEL:
                            process.env.ANTHROPIC_DEFAULT_SONNET_MODEL ??
                            "us.anthropic.claude-sonnet-4-6",
                        ANTHROPIC_DEFAULT_HAIKU_MODEL:
                            process.env.ANTHROPIC_DEFAULT_HAIKU_MODEL ??
                            "us.anthropic.claude-sonnet-4-6",
                        ANTHROPIC_DEFAULT_OPUS_MODEL:
                            process.env.ANTHROPIC_DEFAULT_OPUS_MODEL ??
                            "us.anthropic.claude-sonnet-4-6",
                    },
                },
            })) {
                if (
                    message.type === "result" &&
                    message.subtype === "success"
                ) {
                    finalResult = message.result;
                }
            }

            return { output: finalResult } as ClaudeAgentResult;
        });

        console.log(
            JSON.stringify({
                message: "Claude Agent SDK execution complete",
                result: result.output,
            }),
        );

        return result;
    },
);
