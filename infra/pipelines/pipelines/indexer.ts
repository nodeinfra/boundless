import * as pulumi from "@pulumi/pulumi";
import * as aws from "@pulumi/aws";
import { BOUNDLESS_PROD_DEPLOYMENT_ROLE_ARN, BOUNDLESS_STAGING_DEPLOYMENT_ROLE_ARN } from "../accountConstants";
import { BasePipelineArgs } from "./base";

interface IndexerPipelineArgs extends BasePipelineArgs { }

// The name of the app that we are deploying. Must match the name of the directory in the infra directory.
const APP_NAME = "indexer";
// The branch that we should deploy from on push.
const BRANCH_NAME = "zeroecco/rm_beta";
// The buildspec for the CodeBuild project that deploys our Pulumi stacks to the staging and prod accounts.
// Note in pre-build we assume the deployment role for the given account before running pulumi commands, so
// that we deploy to the target account.
const BUILD_SPEC = `
    version: 0.2

    env:
      git-credential-helper: yes
    
    phases:
      pre_build:
        commands:
          - echo Assuming role $DEPLOYMENT_ROLE_ARN
          - ASSUMED_ROLE=$(aws sts assume-role --role-arn $DEPLOYMENT_ROLE_ARN --role-session-name Deployment --output text | tail -1)
          - export AWS_ACCESS_KEY_ID=$(echo $ASSUMED_ROLE | awk '{print $2}')
          - export AWS_SECRET_ACCESS_KEY=$(echo $ASSUMED_ROLE | awk '{print $4}')
          - export AWS_SESSION_TOKEN=$(echo $ASSUMED_ROLE | awk '{print $5}')
          - curl -fsSL https://get.pulumi.com/ | sh
          - export PATH=$PATH:$HOME/.pulumi/bin
          - pulumi login --non-interactive "s3://boundless-pulumi-state?region=us-west-2&awssdk=v2"
          - git submodule update --init --recursive
          - echo $DOCKER_PAT > docker_token.txt
          - cat docker_token.txt | docker login -u $DOCKER_USERNAME --password-stdin
          - curl https://sh.rustup.rs -sSf | sh -s -- -y
          - . "$HOME/.cargo/env"
          - curl -fsSL https://cargo-lambda.info/install.sh | sh -s -- -y
          - . "$HOME/.cargo/env"
          - npm install -g @ziglang/cli
          - ls -lt
      build:
        commands:
          - cd infra/$APP_NAME
          - pulumi install
          - echo "Tearing down stack $STACK_NAME"
          - pulumi stack select $STACK_NAME
          - pulumi down --yes
    `;

export class IndexerPipeline extends pulumi.ComponentResource {
  constructor(name: string, args: IndexerPipelineArgs, opts?: pulumi.ComponentResourceOptions) {
    super(`boundless:pipelines:${APP_NAME}Pipeline`, name, args, opts);

    const { connection, artifactBucket, role, githubToken, dockerUsername, dockerToken, slackAlertsTopicArn } = args;

    // These tokens are needed to avoid being rate limited by Github/Docker during the build process.
    const githubTokenSecret = new aws.secretsmanager.Secret(`${APP_NAME}-ghToken`);
    const dockerTokenSecret = new aws.secretsmanager.Secret(`${APP_NAME}-dockerToken`);

    new aws.secretsmanager.SecretVersion(`${APP_NAME}-ghTokenVersion`, {
      secretId: githubTokenSecret.id,
      secretString: githubToken,
    });

    new aws.secretsmanager.SecretVersion(`${APP_NAME}-dockerTokenVersion`, {
      secretId: dockerTokenSecret.id,
      secretString: dockerToken,
    });

    new aws.iam.RolePolicy(`${APP_NAME}-build-secrets`, {
      role: role.id,
      policy: {
        Version: '2012-10-17',
        Statement: [
          {
            Effect: 'Allow',
            Action: ['secretsmanager:GetSecretValue', 'ssm:GetParameters'],
            Resource: [githubTokenSecret.arn, dockerTokenSecret.arn],
          },
        ],
      },
    });

    const stagingDeploymentEthSepolia = new aws.codebuild.Project(
      `${APP_NAME}-staging-11155111-build`,
      this.codeBuildProjectArgs(APP_NAME, "staging-11155111", role, BOUNDLESS_STAGING_DEPLOYMENT_ROLE_ARN, dockerUsername, dockerTokenSecret, githubTokenSecret),
      { dependsOn: [role] }
    );

    const stagingDeploymentBaseSepolia = new aws.codebuild.Project(
      `${APP_NAME}-staging-84532-build`,
      this.codeBuildProjectArgs(APP_NAME, "staging-84532", role, BOUNDLESS_STAGING_DEPLOYMENT_ROLE_ARN, dockerUsername, dockerTokenSecret, githubTokenSecret),
      { dependsOn: [role] }
    );

    const prodDeploymentEthSepolia = new aws.codebuild.Project(
      `${APP_NAME}-prod-11155111-build`,
      this.codeBuildProjectArgs(APP_NAME, "prod-11155111", role, BOUNDLESS_PROD_DEPLOYMENT_ROLE_ARN, dockerUsername, dockerTokenSecret, githubTokenSecret),
      { dependsOn: [role] }
    );

    const prodDeploymentBaseMainnet = new aws.codebuild.Project(
      `${APP_NAME}-prod-8453-build`,
      this.codeBuildProjectArgs(APP_NAME, "prod-8453", role, BOUNDLESS_PROD_DEPLOYMENT_ROLE_ARN, dockerUsername, dockerTokenSecret, githubTokenSecret),
      { dependsOn: [role] }
    );

    const prodDeploymentBaseSepolia = new aws.codebuild.Project(
      `${APP_NAME}-prod-84532-build`,
      this.codeBuildProjectArgs(APP_NAME, "prod-84532", role, BOUNDLESS_PROD_DEPLOYMENT_ROLE_ARN, dockerUsername, dockerTokenSecret, githubTokenSecret),
      { dependsOn: [role] }
    );

    const pipeline = new aws.codepipeline.Pipeline(`${APP_NAME}-pipeline`, {
      pipelineType: "V2",
      artifactStores: [{
        type: "S3",
        location: artifactBucket.bucket
      }],
      stages: [
        {
          name: "Github",
          actions: [{
            name: "Github",
            category: "Source",
            owner: "AWS",
            provider: "CodeStarSourceConnection",
            version: "1",
            outputArtifacts: ["source_output"],
            configuration: {
              ConnectionArn: connection.arn,
              FullRepositoryId: "boundless-xyz/boundless",
              BranchName: BRANCH_NAME,
              OutputArtifactFormat: "CODEBUILD_CLONE_REF"
            },
          }],
        },
        {
          name: "DeployStaging",
          actions: [
            {
              name: "DeployStagingEthSepolia",
              category: "Build",
              owner: "AWS",
              provider: "CodeBuild",
              version: "1",
              runOrder: 1,
              configuration: {
                ProjectName: stagingDeploymentEthSepolia.name
              },
              outputArtifacts: ["staging_output_eth_sepolia"],
              inputArtifacts: ["source_output"],
            },
            {
              name: "DeployStagingBaseSepolia",
              category: "Build",
              owner: "AWS",
              provider: "CodeBuild",
              version: "1",
              runOrder: 1,
              configuration: {
                ProjectName: stagingDeploymentBaseSepolia.name
              },
              outputArtifacts: ["staging_output_base_sepolia"],
              inputArtifacts: ["source_output"],
            }
          ]
        },
        {
          name: "DeployProduction",
          actions: [
            {
              name: "ApproveDeployToProduction",
              category: "Approval",
              owner: "AWS",
              provider: "Manual",
              version: "1",
              runOrder: 1,
              configuration: {}
            },
            {
              name: "DeployProductionBaseSepolia",
              category: "Build",
              owner: "AWS",
              provider: "CodeBuild",
              version: "1",
              runOrder: 2,
              configuration: {
                ProjectName: prodDeploymentBaseSepolia.name
              },
              outputArtifacts: ["production_output_base_sepolia"],
              inputArtifacts: ["source_output"],
            },
            {
              name: "DeployProductionEthSepolia",
              category: "Build",
              owner: "AWS",
              provider: "CodeBuild",
              version: "1",
              runOrder: 2,
              configuration: {
                ProjectName: prodDeploymentEthSepolia.name
              },
              outputArtifacts: ["production_output_eth_sepolia"],
              inputArtifacts: ["source_output"],
            },
            {
              name: "DeployProductionBaseMainnet",
              category: "Build",
              owner: "AWS",
              provider: "CodeBuild",
              version: "1",
              runOrder: 2,
              configuration: {
                ProjectName: prodDeploymentBaseMainnet.name
              },
              outputArtifacts: ["production_output_base_mainnet"],
              inputArtifacts: ["source_output"],
            }
          ]
        }
      ],
      triggers: [
        {
          providerType: "CodeStarSourceConnection",
          gitConfiguration: {
            sourceActionName: "Github",
            pushes: [
              {
                branches: {
                  includes: [BRANCH_NAME],
                },
              },
            ],
          },
        },
      ],
      name: `${APP_NAME}-pipeline`,
      roleArn: role.arn,
    });

    new aws.codestarnotifications.NotificationRule(`${APP_NAME}-pipeline-notifications`, {
      name: `${APP_NAME}-pipeline-notifications`,
      eventTypeIds: [
        "codepipeline-pipeline-manual-approval-succeeded",
        "codepipeline-pipeline-action-execution-failed",
      ],
      resource: pipeline.arn,
      detailType: "FULL",
      targets: [
        {
          address: slackAlertsTopicArn.apply(arn => arn),
        },
      ],
    });
  }

  private codeBuildProjectArgs(
    appName: string,
    stackName: string,
    role: aws.iam.Role,
    serviceAccountRoleArn: string,
    dockerUsername: string,
    dockerTokenSecret: aws.secretsmanager.Secret,
    githubTokenSecret: aws.secretsmanager.Secret
  ): aws.codebuild.ProjectArgs {
    return {
      buildTimeout: 60,
      description: `Deployment for ${APP_NAME}`,
      serviceRole: role.arn,
      environment: {
        computeType: "BUILD_GENERAL1_MEDIUM",
        image: "aws/codebuild/standard:7.0",
        type: "LINUX_CONTAINER",
        privilegedMode: true,
        environmentVariables: [
          {
            name: "DEPLOYMENT_ROLE_ARN",
            type: "PLAINTEXT",
            value: serviceAccountRoleArn
          },
          {
            name: "STACK_NAME",
            type: "PLAINTEXT",
            value: stackName
          },
          {
            name: "APP_NAME",
            type: "PLAINTEXT",
            value: appName
          },
          {
            name: "GITHUB_TOKEN",
            type: "SECRETS_MANAGER",
            value: githubTokenSecret.name
          },
          {
            name: "DOCKER_USERNAME",
            type: "PLAINTEXT",
            value: dockerUsername
          },
          {
            name: "DOCKER_PAT",
            type: "SECRETS_MANAGER",
            value: dockerTokenSecret.name
          }
        ]
      },
      artifacts: { type: "CODEPIPELINE" },
      source: {
        type: "CODEPIPELINE",
        buildspec: BUILD_SPEC
      }
    }
  }
}
