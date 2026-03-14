#!/usr/bin/env node
import * as cdk from "aws-cdk-lib";
import { loadEnvFile } from "../lib/utils/load-env";
import { getEnvironment } from "../lib/config/environments";
import { getNamePrefixed } from "../lib/utils/prefix";
import { LabStack } from "../lib/stacks/lab";

// Load .env file
loadEnvFile();

/**
 * Settings
 */
const AWS_ACCOUNT = process.env.AWS_ACCOUNT || process.env.CDK_DEFAULT_ACCOUNT;
const AWS_REGION = process.env.AWS_REGION || process.env.CDK_DEFAULT_REGION;
const PROJECT_OWNER = process.env.PROJECT_OWNER
    ? { OWNER: process.env.PROJECT_OWNER }
    : null;

const environment = getEnvironment();
const app = new cdk.App();
const env = {
    account: AWS_ACCOUNT,
    region: AWS_REGION,
};
const tags = {
    ...PROJECT_OWNER,
    APP: environment.appName,
    Name: environment.appName,
};

/**
 * Stacks
 */
const labStackName = getNamePrefixed("rust-durable-functions-lab", environment);
const _labStack = new LabStack(app, "RustDurableFunctionsLab", {
    stackName: labStackName,
    environment,
    env,
    tags: {
        ...tags,
        STACK: labStackName,
    },
});

app.synth();
