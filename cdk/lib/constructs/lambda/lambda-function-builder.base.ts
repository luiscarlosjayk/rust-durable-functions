import * as cdk from "aws-cdk-lib";
import { FoundationModelIdentifier } from "aws-cdk-lib/aws-bedrock";
import * as dynamoDb from "aws-cdk-lib/aws-dynamodb";
import * as ec2 from "aws-cdk-lib/aws-ec2";
import * as iam from "aws-cdk-lib/aws-iam";
import * as lambda from "aws-cdk-lib/aws-lambda";
import * as logs from "aws-cdk-lib/aws-logs";
import * as s3 from "aws-cdk-lib/aws-s3";
import * as secretsManager from "aws-cdk-lib/aws-secretsmanager";
import * as sqs from "aws-cdk-lib/aws-sqs";
import { Construct } from "constructs";
import { Environment } from "../../types/environment";
import { NotUndefined } from "../../types/utility";
import { getNamePrefixed } from "../../utils/prefix";
import { StringParameterStoreConstruct } from "../parameter-store-string";
import * as sns from "aws-cdk-lib/aws-sns";

export interface IFunctionProps {
    name: string;
    environment: Environment;
}

export abstract class LambdaFunctionBuilder<
    T extends lambda.IFunction,
> extends Construct {
    protected _id: string;
    protected _name: string;
    protected _lambdaName: string;
    protected _environment: Environment;
    protected _logGroup?: lambda.FunctionProps["logGroup"];
    protected _role: NotUndefined<lambda.FunctionProps, "role">;
    protected _lambda: T;
    protected _duration?: lambda.FunctionProps["timeout"];
    protected _memorySize?: lambda.FunctionProps["memorySize"];
    protected _concurrency?: lambda.FunctionProps["reservedConcurrentExecutions"];
    protected _environmentVariables: NotUndefined<
        lambda.FunctionProps,
        "environment"
    >;
    protected _vpc?: lambda.FunctionProps["vpc"];
    protected _vpcSubnets?: lambda.FunctionProps["vpcSubnets"];
    protected _securityGroups?: lambda.FunctionProps["securityGroups"];
    protected _layers?: lambda.FunctionProps["layers"];
    protected _runtime: lambda.Runtime;
    protected _allowPublicSubnet?: boolean;
    protected _systemLogLevel: lambda.SystemLogLevel;
    protected _applicationLogLevel: lambda.ApplicationLogLevel;
    protected _loggingFormat: lambda.LoggingFormat;
    protected _durableConfig: lambda.DurableConfig;

    abstract build(): T;

    constructor(
        scope: Construct,
        id: string,
        name: string,
        environment: Environment,
    ) {
        super(scope, id);

        this._id = id;
        this._name = name;
        this._environment = environment;
        this._lambdaName = getNamePrefixed(name, environment);
        this._environmentVariables = {};
        this._role = new iam.Role(this, `Role${id}`, {
            assumedBy: new iam.CompositePrincipal(
                new iam.ServicePrincipal("lambda.amazonaws.com"),
            ),
            managedPolicies: [
                iam.ManagedPolicy.fromAwsManagedPolicyName(
                    "service-role/AWSLambdaVPCAccessExecutionRole",
                ),
            ],
        });
        this.withLoggingFormat(lambda.LoggingFormat.JSON);

        const defaultPolicyStatements =
            this.createDefaultLambdaPolicyStatementProps();
        this._role.attachInlinePolicy(
            new iam.Policy(this, `DefaultPolicy${id}`, {
                statements: defaultPolicyStatements.map(
                    ({ effect, resources, actions }) =>
                        new iam.PolicyStatement({
                            effect,
                            resources,
                            actions,
                        }),
                ),
            }),
        );

        return this;
    }

    withLogGroup(name?: string): this {
        if (this._logGroup) {
            throw new Error("Log group already set");
        }

        const logGroupName = name ?? `/aws/lambda/${this._lambdaName}`;
        this._logGroup = new logs.LogGroup(this, `LogGroup${this._id}`, {
            logGroupName,
            removalPolicy: cdk.RemovalPolicy.DESTROY,
            retention: logs.RetentionDays.ONE_WEEK,
        });

        return this;
    }

    withSystemLogLevel(level: lambda.SystemLogLevel): this {
        this._systemLogLevel = level;

        return this;
    }

    withApplicationLogLevel(level: lambda.ApplicationLogLevel): this {
        this._applicationLogLevel = level;

        return this;
    }

    withLoggingFormat(format: lambda.LoggingFormat): this {
        this._loggingFormat = format;

        return this;
    }

    withDuration(duration: cdk.Duration): this {
        this._duration = duration;
        return this;
    }

    withSecret(
        secret: secretsManager.ISecret,
        environmentVariable?: string,
    ): this {
        secret.grantRead(this._role);

        if (environmentVariable) {
            this.withEnvironmentVariable(
                environmentVariable,
                secret.secretName,
            );
        }

        return this;
    }

    withSQS(queue: sqs.IQueue, environmentVariable?: string): this {
        queue.grantConsumeMessages(this._role);
        queue.grantSendMessages(this._role);

        if (environmentVariable) {
            this.withEnvironmentVariable(environmentVariable, queue.queueUrl);
        }

        return this;
    }

    withSNS(topic: sns.ITopic, environmentVariable?: string): this {
        topic.grantPublish(this._role);

        if (environmentVariable) {
            this.withEnvironmentVariable(environmentVariable, topic.topicArn);
        }

        return this;
    }

    withDynamoDBTable(
        table: dynamoDb.Table,
        environmentVariable?: string,
    ): this {
        table.grantReadWriteData(this._role);

        if (environmentVariable) {
            this.withEnvironmentVariable(environmentVariable, table.tableName);
        }

        return this;
    }
    
    withDynamoDBTableV2(
        table: dynamoDb.TableV2,
        environmentVariable?: string,
    ): this {
        table.grantReadWriteData(this._role);

        if (environmentVariable) {
            this.withEnvironmentVariable(environmentVariable, table.tableName);
        }

        return this;
    }

    withBucket(bucket: s3.IBucket, environmentVariable?: string): this {
        bucket.grantReadWrite(this._role);
        bucket.grantPut(this._role);

        if (environmentVariable) {
            this.withEnvironmentVariable(
                environmentVariable,
                bucket.bucketName,
            );
        }

        return this;
    }

    withManagedPolicy(managedPolicy: iam.IManagedPolicy): this {
        this._role.addManagedPolicy(managedPolicy);

        return this;
    }

    attachInlinePolicy(policy: iam.Policy): this {
        this._role.attachInlinePolicy(policy);
        return this;
    }

    withMemorySize(memorySizeInMB: number): this {
        this._memorySize = memorySizeInMB;
        return this;
    }

    withConcurrency(reservedConcurrentExecutions: number): this {
        this._concurrency = reservedConcurrentExecutions;
        return this;
    }

    withEnvironmentVariables(
        environmentVariables: NotUndefined<lambda.FunctionProps, "environment">,
    ): this {
        this._environmentVariables = {
            ...this._environmentVariables,
            ...environmentVariables,
        };
        return this;
    }

    withVpc(
        vpc: ec2.IVpc,
        securityGroups: ec2.ISecurityGroup | ec2.ISecurityGroup[],
        vpcSubnets?: ec2.SubnetSelection,
    ): this {
        vpcSubnets = vpcSubnets ?? {
            subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS,
        };

        securityGroups = Array.isArray(securityGroups)
            ? securityGroups
            : [securityGroups];

        this._vpc = vpc;
        this._vpcSubnets = vpcSubnets;
        this._securityGroups = securityGroups;

        if (this._vpcSubnets?.subnetType === ec2.SubnetType.PUBLIC) {
            this._allowPublicSubnet = true;
        }

        return this;
    }

    withRuntime(runtime: lambda.Runtime): this {
        this._runtime = runtime;
        return this;
    }

    withEnvironmentVariable(name: string, value: string): this {
        this._environmentVariables[name] = value;
        return this;
    }

    withLayers(layers: NotUndefined<lambda.FunctionProps, "layers">): this {
        this._layers = layers;
        return this;
    }

    withDurableConfig(config: lambda.DurableConfig): this {
        this._durableConfig = config;
        return this;
    }

    withParameterStore(
        parameterStoreString: StringParameterStoreConstruct,
    ): this {
        parameterStoreString.parameter.grantRead(this._role);
        this.withEnvironmentVariable(
            parameterStoreString.envName,
            parameterStoreString.parameter.parameterName,
        );

        return this;
    }

    withKnowledgeBase(
        knowledgeBaseId: string,
        environmentVariable?: string,
    ): this {
        this.createBedrockKnowledegeBasePolicyStatementProps([knowledgeBaseId]);

        if (environmentVariable) {
            this.withEnvironmentVariable(environmentVariable, knowledgeBaseId);
        }

        return this;
    }

    withBedrockFoundationalModels(
        bedrockModelIdentifiers: FoundationModelIdentifier[],
        id: string = `BedrockFoundationalModelsPolicy${this._id}`,
    ): this {
        const policyStatementProps =
            this.createBedrockFoundationModelPolicyStatementProps(
                bedrockModelIdentifiers,
            );
        const policy = new iam.Policy(this, id, {
            statements: policyStatementProps.map(
                ({ effect, resources, actions }) =>
                    new iam.PolicyStatement({
                        effect,
                        resources,
                        actions,
                    }),
            ),
        });
        this.attachInlinePolicy(policy);

        return this;
    }

    get role(): iam.IRole {
        return this._role;
    }

    protected createDefaultLambdaPolicyStatementProps(): iam.PolicyStatementProps[] {
        return [
            {
                effect: iam.Effect.ALLOW,
                resources: ["*"],
                actions: [
                    "logs:CreateLogGroup",
                    "logs:CreateLogStream",
                    "logs:DescribeLogGroups",
                    "logs:DescribeLogStreams",
                    "logs:PutLogEvents",
                ],
            },
            {
                effect: iam.Effect.ALLOW,
                resources: ["*"],
                actions: [
                    "ec2:DescribeNetworkInterfaces",
                    "ec2:DetachNetworkInterface",
                    "ec2:CreateNetworkInterface",
                    "ec2:DeleteNetworkInterface",
                    "ec2:DescribeInstances",
                    "ec2:AttachNetworkInterface",
                ],
            },
        ];
    }

    protected createBedrockFoundationModelPolicyStatementProps(
        bedrockModelIdentifiers?: FoundationModelIdentifier[],
        actions?: string[],
    ): iam.PolicyStatementProps[] {
        if (!bedrockModelIdentifiers) {
            return [];
        }

        actions ??= [
            // Default actions if none are passed
            "bedrock:InvokeModel",
            "bedrock:InvokeModelWithResponseStream",
        ];
        return bedrockModelIdentifiers.map((model) => {
            return {
                effect: iam.Effect.ALLOW,
                resources: [
                    `arn:aws:bedrock:${cdk.Aws.REGION}::foundation-model/${model.modelId}`,
                ],
                actions,
            };
        });
    }

    protected createBedrockKnowledegeBasePolicyStatementProps(
        knowledgeBaseIds?: string[],
        actions?: string[],
    ): iam.PolicyStatementProps[] {
        if (!knowledgeBaseIds) {
            return [];
        }

        actions ??= [
            // Default actions if none are passed
            "bedrock:InvokeAgent",
            "bedrock:InvokeModelWithResponseStream",
            "bedrock:Retrieve",
            "bedrock:RetrieveAndGenerate",
            "bedrock:StartIngestionJob",
            "bedrock:StopIngestionJob",
            "bedrock:ListIngestionJobs",
        ];
        return knowledgeBaseIds.map((knowledgeBaseId) => {
            return {
                effect: iam.Effect.ALLOW,
                resources: [
                    `arn:aws:bedrock:${cdk.Aws.REGION}:${cdk.Aws.ACCOUNT_ID}:knowledge-base/${knowledgeBaseId}`,
                ],
                actions,
            };
        });
    }
}
