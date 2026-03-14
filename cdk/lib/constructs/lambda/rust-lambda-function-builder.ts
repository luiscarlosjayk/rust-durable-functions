import * as cdk from "aws-cdk-lib";
import * as lambda from "aws-cdk-lib/aws-lambda";
import * as rustLambda from "cargo-lambda-cdk";
import { Construct } from "constructs";
import * as nodePath from "node:path";
import { LambdaFunctionBuilder } from "./lambda-function-builder.base";
import type { Environment } from "../../types/environment";
import { RUST_BASEPATH } from "../../utils/constants";

export interface RustLambdaFunctionProps {
    name: string;
    environment: Environment;
}

export class RustLambdaFunctionBuilder extends LambdaFunctionBuilder<rustLambda.RustFunction> {
    protected _manifestPath: string;
    protected _bundling?: rustLambda.BundlingOptions;
    protected _binaryName?: string;
    protected _customRuntime: rustLambda.RustFunctionProps["runtime"];

    constructor(scope: Construct, id: string, props: RustLambdaFunctionProps) {
        super(scope, id, props.name, props.environment);

        // Defaults
        this.withManifest(this._name);
    }

    withManifest(path: string, basePath?: string): this {
        basePath = basePath ?? RUST_BASEPATH;
        this._manifestPath = nodePath.join(basePath, `${path}/Cargo.toml`);
        return this;
    }

    withBundling(bundling: rustLambda.BundlingOptions): this {
        this._bundling = bundling;
        return this;
    }

    withBinaryName(binaryName: string): this {
        this._binaryName = binaryName;
        return this;
    }

    build() {
        if (!this._manifestPath) {
            throw "Expected manifestPath to be defined.";
        }

        this._lambda = new rustLambda.RustFunction(this, "RustFunction", {
            functionName: this._lambdaName,
            manifestPath: this._manifestPath,
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
            binaryName: this._binaryName,
            runtime: this._customRuntime,
            systemLogLevelV2: this._systemLogLevel,
            applicationLogLevelV2: this._applicationLogLevel,
            loggingFormat: this._loggingFormat,
            durableConfig: this._durableConfig,
        });

        return this._lambda;
    }
}
