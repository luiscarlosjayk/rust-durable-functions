export type EnvironmentName = "local" | "ci" | "dev" | "qa" | "stage" | "prod";
export type LogLevel = "debug" | "info" | "warn" | "error";
export type Environment = {
    orgName: string;
    envName: EnvironmentName;
    appName: string;
    region: EnvironmentRegion;
    provisionedConcurrencyEnabled: boolean;
    logLevel: LogLevel;
    
};
export type EnvironmentRegion = "us-east-1" | "us-west-2";
