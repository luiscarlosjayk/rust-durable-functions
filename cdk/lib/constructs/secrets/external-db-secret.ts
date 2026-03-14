import * as cdk from 'aws-cdk-lib';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import { Construct } from 'constructs';
import { Environment } from '../../types/environment';
import { getNamePrefixed } from '../../utils/prefix';

export interface ExternalDBSecretProps {
    environment: Environment;
    secretName: string;
    host: string;
    dbport: number;
    username: string;
    password: string;
    dbname: string;
}

export class ExternalDBSecret extends Construct {
    public readonly secret: secretsmanager.Secret;

    constructor(scope: Construct, id: string, props: ExternalDBSecretProps) {
        super(scope, id);

        const { environment, host, dbport, username, password, dbname, secretName } = props;

        this.secret = new secretsmanager.Secret(this, `${secretName}-secret`, {
            secretName: getNamePrefixed(secretName, environment),
            description: `${secretName} Secret`,
            secretObjectValue: {
                host: cdk.SecretValue.unsafePlainText(host),
                dbport: cdk.SecretValue.unsafePlainText(dbport.toString()),
                username: cdk.SecretValue.unsafePlainText(username),
                password: cdk.SecretValue.unsafePlainText(password),
                dbname: cdk.SecretValue.unsafePlainText(dbname),
            },
        });
    }

    public getSecretName(): string {
        return this.secret.secretName;
    }

    public getSecretArn(): string {
        return this.secret.secretArn;
    }
    
}