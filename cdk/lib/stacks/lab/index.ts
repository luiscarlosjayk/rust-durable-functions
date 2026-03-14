import * as cdk from "aws-cdk-lib";
import * as dynamodb from "aws-cdk-lib/aws-dynamodb";
import * as iam from "aws-cdk-lib/aws-iam";
import * as lambda from "aws-cdk-lib/aws-lambda";
import { Construct } from "constructs";
import { getNamePrefixed } from "../../utils/prefix";
import { RustLambdaFunctionBuilder } from "../../constructs/lambda/rust-lambda-function-builder";
import { Environment } from "../../types/environment";

export interface LabStackProps extends cdk.StackProps {
    environment: Environment;
}

export class LabStack extends cdk.Stack {
    constructor(scope: Construct, id: string, props: LabStackProps) {
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
         * Lambda Functions
         */
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

        /**
         * Callback Sender Lambda (standard, non-durable)
         */
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
    }
}
