import { Construct } from 'constructs';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import { Environment } from '../types/environment';
import { getSSMPrefixed } from '../utils/prefix';

export type StringParameterStoreProps = {
    name: string;
    environment: Environment;
    value: string;
};

export class StringParameterStoreConstruct extends Construct {
    readonly parameter: ssm.StringParameter;
    readonly envName: string;

    constructor(scope: Construct, id: string, props: StringParameterStoreProps) {
        super(scope, id);

        this.envName = props.name.toUpperCase().replace('-', '_');
        const parameterName = getSSMPrefixed(props.name, props.environment);
        this.parameter = new ssm.StringParameter(this, `Parameter${id}`, {
            parameterName,
            stringValue: props.value,
            tier: ssm.ParameterTier.STANDARD,
            simpleName: false,
        });
    }
}
