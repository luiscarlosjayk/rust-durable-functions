import * as cdk from "aws-cdk-lib";
import { FoundationModelIdentifier } from "aws-cdk-lib/aws-bedrock";
import * as dynamodb from "aws-cdk-lib/aws-dynamodb";
import * as iam from "aws-cdk-lib/aws-iam";
import * as lambda from "aws-cdk-lib/aws-lambda";
import * as s3 from "aws-cdk-lib/aws-s3";
import * as sns from "aws-cdk-lib/aws-sns";
import { Construct } from "constructs";
import { getNamePrefixed } from "../utils/prefix";
import { RustLambdaFunctionBuilder } from "../constructs/lambda/rust-lambda-function-builder";
import { Environment } from "../types/environment";

export interface SimpleLabStackProps extends cdk.StackProps {
    environment: Environment;
}

export class SimpleLabStack extends cdk.Stack {
    constructor(scope: Construct, id: string, props: SimpleLabStackProps) {
        super(scope, id, props);

        const { environment } = props;

        /**
         * Settings
         */
        const removalPolicy =
            environment.envName === "prod"
                ? cdk.RemovalPolicy.RETAIN_ON_UPDATE_OR_DELETE
                : cdk.RemovalPolicy.DESTROY;

        /**
         * DynamoDB Tables
         */
        const ordersTable = new dynamodb.TableV2(this, "OrdersTable", {
            partitionKey: {
                name: "PK",
                type: dynamodb.AttributeType.STRING,
            },
            removalPolicy,
        });

        /**
         * S3 Buckets
         */
        const reportsBucket = new s3.Bucket(this, "ReportsBucket", {
            autoDeleteObjects: true,
            removalPolicy: cdk.RemovalPolicy.DESTROY,
        });

        /**
         * SNS Topics
         */
        const notificationsTopic = new sns.Topic(this, "NotificationsTopic");

        /**
         * Lambda Functions
         */

        // Orders Processor Lambda (durable)
        const ordersProcessorLambda = new RustLambdaFunctionBuilder(
            this,
            "OrderProcessorLambda",
            {
                name: "order-processor",
                environment,
            },
        )
            .withLogGroup()
            .withManifest("order-processor") // Location within src/lambda/rust
            .withMemorySize(256)
            .withEnvironmentVariables({
                RUST_LOG: environment.logLevel,
                RUST_ENV: environment.envName,
                AWS_LAMBDA_EXEC_WRAPPER: "/var/task/bootstrap",
            })
            .withApplicationLogLevel(lambda.ApplicationLogLevel.DEBUG)
            .withDuration(cdk.Duration.minutes(1))
            .withDurableConfig({
                executionTimeout: cdk.Duration.hours(1),
                retentionPeriod: cdk.Duration.days(3),
            })
            .withDynamoDBTableV2(ordersTable, "ORDERS_TABLE")
            // https://docs.aws.amazon.com/lambda/latest/dg/durable-security.html
            .withManagedPolicy(
                iam.ManagedPolicy.fromAwsManagedPolicyName(
                    "service-role/AWSLambdaBasicDurableExecutionRolePolicy",
                ),
            )
            .build();

        // L1 escape hatch: override runtime so CloudFormation accepts DurableConfig
        const cfnFunc = ordersProcessorLambda.node
            .defaultChild as cdk.CfnResource;
        cfnFunc.addPropertyOverride("Runtime", "nodejs24.x");
        cfnFunc.addPropertyOverride("Handler", "index.handler");

        ordersProcessorLambda.applyRemovalPolicy(removalPolicy);

        // Callback Sender Lambda (standard, non-durable)
        const callbackSenderLambda = new RustLambdaFunctionBuilder(
            this,
            "CallbackSenderLambda",
            {
                name: "callback-sender",
                environment,
            },
        )
            .withLogGroup()
            .withManifest("callback-sender")
            .withMemorySize(256)
            .withEnvironmentVariables({
                RUST_LOG: environment.logLevel,
                RUST_ENV: environment.envName,
            })
            .withDynamoDBTableV2(ordersTable, "ORDERS_TABLE")
            .withDuration(cdk.Duration.seconds(30))
            .attachInlinePolicy(
                new iam.Policy(this, "CallbackSenderDurablePolicy", {
                    statements: [
                        new iam.PolicyStatement({
                            effect: iam.Effect.ALLOW,
                            actions: [
                                "lambda:SendDurableExecutionCallbackSuccess",
                            ],
                            resources: [
                                ordersProcessorLambda.functionArn,
                                `${ordersProcessorLambda.functionArn}:*`,
                            ],
                        }),
                    ],
                }),
            )
            .build();

        const callbackUrl = callbackSenderLambda.addFunctionUrl({
            authType: lambda.FunctionUrlAuthType.NONE,
        });

        callbackSenderLambda.applyRemovalPolicy(removalPolicy);

        // Report Orchestrator Lambda (Durable) — Bedrock agent with tool loop
        const reportOrchestratorLambda = new RustLambdaFunctionBuilder(
            this,
            "ReportOrchestratorLambda",
            {
                name: "report-orchestrator",
                environment,
            },
        )
            .withLogGroup()
            .withManifest("report-orchestrator")
            .withMemorySize(512)
            .withEnvironmentVariables({
                RUST_LOG: environment.logLevel,
                RUST_ENV: environment.envName,
                AWS_LAMBDA_EXEC_WRAPPER: "/var/task/bootstrap",
                BEDROCK_MODEL_ID: "us.anthropic.claude-sonnet-4-6",
            })
            .withApplicationLogLevel(lambda.ApplicationLogLevel.DEBUG)
            .withDuration(cdk.Duration.minutes(1))
            .withDurableConfig({
                executionTimeout: cdk.Duration.hours(1),
                retentionPeriod: cdk.Duration.days(3),
            })
            .withSNS(notificationsTopic, "NOTIFICATIONS_TOPIC_ARN")
            .withBedrockFoundationalModels([
                FoundationModelIdentifier.ANTHROPIC_CLAUDE_SONNET_4_6,
            ])
            .withManagedPolicy(
                iam.ManagedPolicy.fromAwsManagedPolicyName(
                    "service-role/AWSLambdaBasicDurableExecutionRolePolicy",
                ),
            )
            .build();

        // L1 escape hatch for durable config
        const cfnOrchestratorFunc = reportOrchestratorLambda.node
            .defaultChild as cdk.CfnResource;
        cfnOrchestratorFunc.addPropertyOverride("Runtime", "nodejs24.x");
        cfnOrchestratorFunc.addPropertyOverride("Handler", "index.handler");

        reportOrchestratorLambda.applyRemovalPolicy(removalPolicy);

        // Orders Fetcher Lambda (Standard) — Scans DynamoDB, sends callback
        const ordersFetcherLambda = new RustLambdaFunctionBuilder(
            this,
            "OrdersFetcherLambda",
            {
                name: "orders-fetcher",
                environment,
            },
        )
            .withLogGroup()
            .withManifest("orders-fetcher")
            .withMemorySize(256)
            .withEnvironmentVariables({
                RUST_LOG: environment.logLevel,
                RUST_ENV: environment.envName,
            })
            .withApplicationLogLevel(lambda.ApplicationLogLevel.DEBUG)
            .withDynamoDBTableV2(ordersTable, "ORDERS_TABLE")
            .withDuration(cdk.Duration.seconds(30))
            .attachInlinePolicy(
                new iam.Policy(this, "OrdersFetcherDurablePolicy", {
                    statements: [
                        new iam.PolicyStatement({
                            effect: iam.Effect.ALLOW,
                            actions: [
                                "lambda:SendDurableExecutionCallbackSuccess",
                            ],
                            resources: [
                                reportOrchestratorLambda.functionArn,
                                `${reportOrchestratorLambda.functionArn}:*`,
                            ],
                        }),
                    ],
                }),
            )
            .build();

        ordersFetcherLambda.applyRemovalPolicy(removalPolicy);

        // Report Saver Lambda (Standard) — Writes markdown to S3, sends callback
        const reportSaverLambda = new RustLambdaFunctionBuilder(
            this,
            "ReportSaverLambda",
            {
                name: "report-saver",
                environment,
            },
        )
            .withLogGroup()
            .withManifest("report-saver")
            .withMemorySize(256)
            .withEnvironmentVariables({
                RUST_LOG: environment.logLevel,
                RUST_ENV: environment.envName,
            })
            .withApplicationLogLevel(lambda.ApplicationLogLevel.DEBUG)
            .withBucket(reportsBucket, "REPORTS_BUCKET")
            .withDuration(cdk.Duration.seconds(30))
            .attachInlinePolicy(
                new iam.Policy(this, "ReportSaverDurablePolicy", {
                    statements: [
                        new iam.PolicyStatement({
                            effect: iam.Effect.ALLOW,
                            actions: [
                                "lambda:SendDurableExecutionCallbackSuccess",
                            ],
                            resources: [
                                reportOrchestratorLambda.functionArn,
                                `${reportOrchestratorLambda.functionArn}:*`,
                            ],
                        }),
                    ],
                }),
            )
            .build();

        reportSaverLambda.applyRemovalPolicy(removalPolicy);

        /**
         * Post-build: Add env vars to orchestrator (depends on fetcher/saver being created)
         */
        reportOrchestratorLambda.addEnvironment(
            "ORDERS_FETCHER_FUNCTION_NAME",
            ordersFetcherLambda.functionName,
        );
        reportOrchestratorLambda.addEnvironment(
            "REPORT_SAVER_FUNCTION_NAME",
            reportSaverLambda.functionName,
        );

        /**
         * Orchestrator IAM: invoke fetcher and saver
         */
        reportOrchestratorLambda.role!.attachInlinePolicy(
            new iam.Policy(this, "OrchestratorInvokePolicy", {
                statements: [
                    new iam.PolicyStatement({
                        effect: iam.Effect.ALLOW,
                        actions: ["lambda:InvokeFunction"],
                        resources: [
                            ordersFetcherLambda.functionArn,
                            reportSaverLambda.functionArn,
                        ],
                    }),
                ],
            }),
        );

        /**
         * Stack Exports
         */
        new cdk.CfnOutput(this, "ExportWebsiteLambdaArn", {
            exportName: getNamePrefixed("durable-lambda-arn", environment),
            value: ordersProcessorLambda.functionArn,
        });

        new cdk.CfnOutput(this, "ExportCallbackSenderUrl", {
            exportName: getNamePrefixed("callback-sender-url", environment),
            value: callbackUrl.url,
        });

        new cdk.CfnOutput(this, "ExportReportOrchestratorArn", {
            exportName: getNamePrefixed("report-orchestrator-arn", environment),
            value: reportOrchestratorLambda.functionArn,
        });

        new cdk.CfnOutput(this, "ExportSNSNotificationsTopicArn", {
            exportName: getNamePrefixed(
                "sns-notifications-topic-arn",
                environment,
            ),
            value: notificationsTopic.topicArn,
        });
    }
}
