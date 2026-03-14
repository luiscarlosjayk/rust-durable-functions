import * as goLambda from "@aws-cdk/aws-lambda-go-alpha";
import * as cdk from "aws-cdk-lib";
import * as lambda from "aws-cdk-lib/aws-lambda";
import { Construct } from "constructs";
import * as nodePath from "node:path";
import { LambdaFunctionBuilder } from "./lambda-function-builder.base";
import type { Environment } from "../../types/environment";
import { GOLANG_BASEPATH } from "../../utils/constants";

export interface GoLambdaFunctionProps {
    name: string;
    environment: Environment;
}

export class GoLambdaFunctionBuilder extends LambdaFunctionBuilder<goLambda.GoFunction> {
    protected _entry: string;
    protected _moduleDir?: string;
    protected _bundling?: goLambda.BundlingOptions;

    constructor(scope: Construct, id: string, props: GoLambdaFunctionProps) {
        super(scope, id, props.name, props.environment);

        // Defaults
        this.withEntry(this._name);
        this.withModuleDir(this._name);
    }

    withEntry(path: string, basePath?: string): this {
        basePath = basePath ?? GOLANG_BASEPATH;
        this._entry = nodePath.join(basePath, path, "main.go");
        return this;
    }

    withModuleDir(path: string, basePath?: string): this {
        basePath = basePath ?? GOLANG_BASEPATH;
        this._moduleDir = nodePath.join(basePath, path, "go.mod");
        return this;
    }

    withBundling(bundling: goLambda.BundlingOptions): this {
        this._bundling = bundling;
        return this;
    }

    build() {
        if (!this._entry) {
            throw "Expected entry to be defined.";
        }

        if (!this._moduleDir) {
            throw "Expected moduleDir to be defined.";
        }

        this._lambda = new goLambda.GoFunction(this, `GoLambda${this._id}`, {
            functionName: this._lambdaName,
            entry: this._entry,
            moduleDir: this._moduleDir,
            timeout: this._duration,
            memorySize: this._memorySize,
            logGroup: this._logGroup,
            environment: this._environmentVariables,
            reservedConcurrentExecutions: this._concurrency,
            architecture: lambda.Architecture.ARM_64,
            role: this._role,
            vpc: this._vpc,
            vpcSubnets: this._vpcSubnets,
            securityGroups: this._securityGroups,
            layers: this._layers,
            bundling: this._bundling,
            allowPublicSubnet: this._allowPublicSubnet,
            runtime: this._runtime,
            systemLogLevelV2: this._systemLogLevel,
            applicationLogLevelV2: this._applicationLogLevel,
            loggingFormat: this._loggingFormat,
            durableConfig: this._durableConfig,
        });

        return this._lambda;
    }
}
