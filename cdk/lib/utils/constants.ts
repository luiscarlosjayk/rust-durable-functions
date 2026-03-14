import { Runtime } from "aws-cdk-lib/aws-lambda";
import { STATUS_CODES } from "node:http";
import * as nodePath from "node:path";

export const SRC_BASEPATH = "../../../src";
export const SRC_PATH = nodePath.join(__dirname, `${SRC_BASEPATH}`);
export const APP_PATH = nodePath.join(SRC_PATH, "app");
export const LAMBDA_BASEPATH = nodePath.join(
    __dirname,
    `${SRC_BASEPATH}/lambda`,
);
export const GOLANG_BASEPATH = `${LAMBDA_BASEPATH}/golang`;
export const RUST_BASEPATH = `${LAMBDA_BASEPATH}/rust`;
export const NODEJS_BASEPATH = `${LAMBDA_BASEPATH}/nodejs`;
export const HTTP = {
    METHOD: {
        GET: "GET",
        POST: "POST",
        PUT: "PUT",
        PATCH: "PATCH",
        DELETE: "DELETE",
        HEAD: "HEAD",
        OPTIONS: "OPTIONS",
        CONNECT: "CONNECT",
        TRACE: "TRACE",
    },
    STATUS_CODE: Object.entries(STATUS_CODES).map(([code, message]) => ({
        code: parseInt(code),
        message,
    })),
} as const;

export const NODEJS_RUNTIME = {
    LATEST: Runtime.NODEJS_LATEST,
    NODEJS_22_X: Runtime.NODEJS_22_X,
    NODEJS_20_X: Runtime.NODEJS_20_X,
};

export const PYTHON_RUNTIME = {
    LATEST: Runtime.determineLatestPythonRuntime,
    PYTHON_3_13: Runtime.PYTHON_3_13,
    PYTHON_3_12: Runtime.PYTHON_3_12,
    PYTHON_3_11: Runtime.PYTHON_3_11,
    PYTHON_3_10: Runtime.PYTHON_3_10,
};
