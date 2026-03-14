import type { Environment } from "../types/environment";

export function getSSMPrefixed(name: string, environment: Environment): string {
    return `/${environment.appName}-${environment.envName}/${name}`.toLowerCase();
}

export function getNamePrefixed(
    name: string,
    environment: Environment,
    separator = "-",
): string {
    return [environment.appName, environment.envName, name]
        .filter((s) => s) // Remove empty strings
        .join(separator)
        .toLowerCase();
}
