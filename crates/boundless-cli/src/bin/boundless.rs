// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The Boundless CLI is a command-line interface for interacting with Boundless.

const CLI_LONG_ABOUT: &str = r#"
The Boundless CLI is a command-line interface for interacting with Boundless.

# Examples

```sh
RPC_URL=https://ethereum-sepolia-rpc.publicnode.com \
boundless account balance 0x3da7206e104f6d5dd070bfe06c5373cc45c3e65c
```

```sh
RPC_URL=https://ethereum-sepolia-rpc.publicnode.com \
PRIVATE_KEY=0x0000000000000000000000000000000000000000000000000000000000000000 \
boundless request submit-offer --wait --input "hello" \
--program-url http://dweb.link/ipfs/bafkreido62tz2uyieb3s6wmixwmg43hqybga2ztmdhimv7njuulf3yug4e
```

# Required options

An Ethereum RPC URL is required via the `RPC_URL` environment variable or the `--rpc-url`
flag. You can use a public RPC endpoint for most operations, but it is best to use an RPC
endpoint that supports events (e.g. Alchemy or Infura).

Sending, fulfilling, and slashing requests requires a signer provided via the `PRIVATE_KEY`
environment variable or `--private-key`. This CLI only supports in-memory private keys as of
this version. Full signer support is available in the SDK."#;

use std::{
    any::Any,
    borrow::Cow,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use alloy::{
    network::Ethereum,
    primitives::{
        utils::{format_ether, format_units, parse_ether, parse_units},
        Address, FixedBytes, TxKind, B256, U256,
    },
    providers::{Provider, ProviderBuilder},
    rpc::types::{TransactionInput, TransactionRequest},
    sol_types::SolValue,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use bonsai_sdk::non_blocking::Client as BonsaiClient;
use boundless_cli::{
    commands::zkc::ZKCCommands, config::ProverConfig, convert_timestamp, DefaultProver,
    OrderFulfilled,
};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::aot::Shell;
use risc0_aggregation::SetInclusionReceiptVerifierParameters;
use risc0_ethereum_contracts::{set_verifier::SetVerifierService, IRiscZeroVerifier};
use risc0_zkvm::{
    compute_image_id, default_executor,
    sha::{Digest, Digestible},
    Journal, SessionInfo,
};
use shadow_rs::shadow;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use url::Url;

use boundless_cli::{commands::povw::PovwCommands, config::GlobalConfig};
use boundless_market::{
    contracts::{
        boundless_market::{BoundlessMarketService, FulfillmentTx, UnlockedRequest},
        FulfillmentData, Offer, Predicate, ProofRequest, RequestInputType, Selector,
    },
    input::GuestEnv,
    request_builder::{OfferParams, RequirementParams},
    selector::ProofType,
    storage::{fetch_url, StorageProvider, StorageProviderConfig},
    Client, Deployment, StandardClient,
};

shadow!(build);

#[derive(Subcommand, Clone, Debug)]
enum Command {
    /// Account management commands
    #[command(subcommand)]
    Account(Box<AccountCommands>),

    /// Proof request commands
    #[command(subcommand)]
    Request(Box<RequestCommands>),

    /// Proof execution commands
    #[command(subcommand)]
    Proving(Box<ProvingCommands>),

    /// Operations on the boundless market
    #[command(subcommand)]
    Ops(Box<OpsCommands>),

    #[command(subcommand)]
    Povw(Box<PovwCommands>),

    #[command(subcommand)]
    Zkc(Box<ZKCCommands>),

    /// Display configuration and environment variables
    Config {},

    /// Print shell completions (e.g. for bash or zsh) to stdout.
    Completions { shell: Shell },
}

#[derive(Subcommand, Clone, Debug)]
enum OpsCommands {
    /// Slash a prover for a given request
    Slash {
        /// The proof request identifier
        request_id: U256,
    },
}

#[derive(Subcommand, Clone, Debug)]
enum AccountCommands {
    /// Deposit funds into the market
    Deposit {
        /// Amount in ether to deposit
        #[clap(value_parser = parse_ether)]
        amount: U256,
    },
    /// Withdraw funds from the market
    Withdraw {
        /// Amount in ether to withdraw
        #[clap(value_parser = parse_ether)]
        amount: U256,
    },
    /// Check the balance of an account in the market
    Balance {
        /// Address to check the balance of;
        /// if not provided, defaults to the wallet address
        address: Option<Address>,
    },
    /// Deposit collateral funds into the market
    DepositCollateral {
        /// Amount to deposit in ZKC.
        amount: String,
    },
    /// Withdraw collateral funds from the market
    WithdrawCollateral {
        /// Amount to withdraw in ZKC.
        amount: String,
    },
    /// Check the collateral balance of an account in the market
    CollateralBalance {
        /// Address to check the balance of;
        /// if not provided, defaults to the wallet address
        address: Option<Address>,
    },
}

#[derive(Subcommand, Clone, Debug)]
enum RequestCommands {
    /// Submit a proof request constructed with the given offer, input, and image
    SubmitOffer(Box<SubmitOfferArgs>),

    /// Submit a fully specified proof request
    Submit {
        /// Path to a YAML file containing the request
        yaml_request: PathBuf,

        /// Wait until the request is fulfilled
        #[clap(short, long, default_value = "false")]
        wait: bool,

        /// Submit the request offchain via the provided order stream service url
        #[clap(short, long)]
        offchain: bool,

        /// Skip preflight check (not recommended)
        #[clap(long, default_value = "false")]
        no_preflight: bool,

        /// Configuration for the StorageProvider to use for uploading programs and inputs.
        #[clap(flatten, next_help_heading = "Storage Provider")]
        storage_config: Box<StorageProviderConfig>,
    },

    /// Get the status of a given request
    Status {
        /// The proof request identifier
        request_id: U256,

        /// The time at which the request expires, in seconds since the UNIX epoch
        expires_at: Option<u64>,
    },

    /// Get the journal and seal for a given request
    GetProof {
        /// The proof request identifier
        request_id: U256,
    },

    /// Verify the proof of the given request against the SetVerifier contract
    VerifyProof {
        /// The proof request identifier
        request_id: U256,

        /// The image id of the original request
        image_id: B256,
    },
}

#[derive(Subcommand, Clone, Debug)]
enum ProvingCommands {
    /// Execute a proof request using the RISC Zero zkVM executor
    Execute {
        /// Path to a YAML file containing the request.
        ///
        /// If provided, the request will be loaded from the given file path.
        #[arg(long, conflicts_with_all = ["request_id", "tx_hash"])]
        request_path: Option<PathBuf>,

        /// The proof request identifier.
        ///
        /// If provided, the request will be fetched from the blockchain.
        #[arg(long, conflicts_with = "request_path")]
        request_id: Option<U256>,

        /// The request digest
        ///
        /// If provided along with request-id, uses the request digest to find the request.
        #[arg(long)]
        request_digest: Option<B256>,

        /// The tx hash of the request submission.
        ///
        /// If provided along with request-id, uses the transaction hash to find the request.
        #[arg(long, conflicts_with = "request_path", requires = "request_id")]
        tx_hash: Option<B256>,
    },
    Benchmark {
        /// Proof request ids to benchmark.
        #[arg(long, value_delimiter = ',')]
        request_ids: Vec<U256>,

        #[clap(flatten, next_help_heading = "Prover")]
        prover_config: ProverConfig,
    },
    /// Fulfill one or more proof requests using the RISC Zero zkVM default prover.
    ///
    /// This command can process multiple requests in a single batch, which is more efficient
    /// than fulfilling requests individually.
    ///
    /// Example usage:
    ///   --request-ids 0x123,0x456,0x789  # Comma-separated list of request IDs
    ///   --request-digests 0xabc,0xdef,0x012  # Optional, must match request_ids length and order
    ///   --tx-hashes 0x111,0x222,0x333  # Optional, must match request_ids length and order
    Fulfill {
        /// The proof requests identifiers (comma-separated list of hex values)
        #[arg(long, value_delimiter = ',')]
        request_ids: Vec<U256>,

        /// The request digests (comma-separated list of hex values).
        /// If provided, must have the same length and order as request_ids.
        #[arg(long, value_delimiter = ',')]
        request_digests: Option<Vec<B256>>,

        /// The tx hash of the requests submissions (comma-separated list of hex values).
        /// If provided, must have the same length and order as request_ids.
        #[arg(long, value_delimiter = ',')]
        tx_hashes: Option<Vec<B256>>,

        /// Withdraw the funds after fulfilling the requests
        #[arg(long, default_value = "false")]
        withdraw: bool,

        #[clap(flatten, next_help_heading = "Prover")]
        prover_config: ProverConfig,
    },

    /// Lock a request in the market
    Lock {
        /// The proof request identifier
        #[arg(long)]
        request_id: U256,

        /// The request digest
        #[arg(long)]
        request_digest: Option<B256>,

        /// The tx hash of the request submission
        #[arg(long)]
        tx_hash: Option<B256>,
    },
}

#[derive(Args, Clone, Debug)]
struct SubmitOfferArgs {
    /// Optional identifier for the request
    id: Option<u32>,

    #[clap(flatten)]
    program: SubmitOfferProgram,

    /// Wait until the request is fulfilled
    #[clap(short, long, default_value = "false")]
    wait: bool,

    /// Submit the request offchain via the provided order stream service url
    #[clap(short, long)]
    offchain: bool,

    /// Use risc0_zkvm::serde to encode the input as a `Vec<u8>`
    #[clap(long)]
    encode_input: bool,

    #[clap(flatten)]
    input: SubmitOfferInput,

    #[clap(flatten)]
    requirements: SubmitOfferRequirements,

    #[clap(flatten, next_help_heading = "Offer")]
    offer_params: OfferParams,

    /// Configuration for the StorageProvider to use for uploading programs and inputs.
    #[clap(flatten, next_help_heading = "Storage Provider")]
    storage_config: StorageProviderConfig,
}

#[derive(Args, Clone, Debug)]
#[group(required = true, multiple = false)]
struct SubmitOfferInput {
    /// Input for the guest, given as a string.
    #[clap(long)]
    input: Option<String>,
    /// Input for the guest, given as a path to a file.
    #[clap(long)]
    input_file: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
#[group(required = true, multiple = false)]
struct SubmitOfferProgram {
    /// Program binary to use as the guest image, given as a path.
    ///
    /// The program will be uploaded to a public URL using the configured storage provider before
    /// the proof request is sent.
    #[clap(short = 'p', long = "program")]
    path: Option<PathBuf>,
    /// Program binary to use as a guest image, given as a public URL.
    ///
    /// This option accepts a pre-uploaded program. If also using small inputs, a storage provider
    /// is not required when using a pre-uploaded program.
    #[clap(long = "program-url")]
    url: Option<Url>,
}

#[derive(Args, Clone, Debug)]
struct SubmitOfferRequirements {
    /// Address of the callback to use in the requirements.
    #[clap(long, requires = "callback_gas_limit")]
    callback_address: Option<Address>,
    /// Gas limit of the callback to use in the requirements.
    #[clap(long, requires = "callback_address")]
    callback_gas_limit: Option<u64>,
    /// Request a groth16 proof (i.e., a Groth16).
    #[clap(long, default_value = "any")]
    proof_type: ProofType,
}

#[derive(Parser, Debug)]
#[clap(author, long_version = build::CLAP_LONG_VERSION, about = "CLI for Boundless", long_about = CLI_LONG_ABOUT)]
struct MainArgs {
    /// Subcommand to run
    #[command(subcommand)]
    command: Command,

    #[command(flatten, next_help_heading = "Global Options")]
    config: GlobalConfig,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = match MainArgs::try_parse() {
        Ok(args) => args,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp {
                // If it's a help request, print the help and exit successfully
                err.print()?;
                return Ok(());
            }
            if err.kind() == clap::error::ErrorKind::DisplayVersion {
                // If it's a version request, print the version and exit successfully
                err.print()?;
                return Ok(());
            }
            return Err(err.into());
        }
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(args.config.log_level.into())
                .from_env_lossy(),
        )
        .init();

    run(&args).await
}

pub(crate) async fn run(args: &MainArgs) -> Result<()> {
    match &args.command {
        Command::Account(account_cmd) => handle_account_command(account_cmd, &args.config).await,
        Command::Request(request_cmd) => handle_request_command(request_cmd, &args.config).await,
        Command::Proving(proving_cmd) => handle_proving_command(proving_cmd, &args.config).await,
        Command::Ops(operation_cmd) => handle_ops_command(operation_cmd, &args.config).await,
        Command::Povw(povw_cmd) => povw_cmd.run(&args.config).await,
        Command::Zkc(zkc_cmd) => zkc_cmd.run(&args.config).await,
        Command::Config {} => handle_config_command(&args.config).await,
        Command::Completions { shell } => generate_shell_completions(shell),
    }
}

fn generate_shell_completions(shell: &Shell) -> Result<()> {
    clap_complete::generate(*shell, &mut MainArgs::command(), "boundless", &mut std::io::stdout());
    Ok(())
}

/// Handle ops-related commands
async fn handle_ops_command(cmd: &OpsCommands, config: &GlobalConfig) -> Result<()> {
    let client = config.build_client_with_signer().await?;
    match cmd {
        OpsCommands::Slash { request_id } => {
            tracing::info!("Slashing prover for request 0x{:x}", request_id);
            client.boundless_market.slash(*request_id).await?;
            tracing::info!("Successfully slashed prover for request 0x{:x}", request_id);
            Ok(())
        }
    }
}

/// Helper function to parse collateral amounts with validation
async fn parse_collateral_amount(
    client: &Client<impl Provider, impl Any, impl Any, impl Any>,
    amount: &str,
) -> Result<(U256, String, String)> {
    let symbol = client.boundless_market.collateral_token_symbol().await?;
    let decimals = client.boundless_market.collateral_token_decimals().await?;
    let parsed_amount =
        parse_units(amount, decimals).map_err(|e| anyhow!("Failed to parse amount: {}", e))?.into();
    if parsed_amount == U256::from(0) {
        bail!("Amount is below the denomination minimum: {}", amount);
    }
    let formatted_amount = format_units(parsed_amount, decimals)?;
    Ok((parsed_amount, formatted_amount, symbol))
}

/// Handle account-related commands
async fn handle_account_command(cmd: &AccountCommands, config: &GlobalConfig) -> Result<()> {
    match cmd {
        AccountCommands::Deposit { amount } => {
            let client = config.build_client_with_signer().await?;
            tracing::info!("Depositing {} ETH into the market", format_ether(*amount));
            client.boundless_market.deposit(*amount).await?;
            tracing::info!("Successfully deposited {} ETH into the market", format_ether(*amount));
            Ok(())
        }
        AccountCommands::Withdraw { amount } => {
            let client = config.build_client_with_signer().await?;
            tracing::info!("Withdrawing {} ETH from the market", format_ether(*amount));
            client.boundless_market.withdraw(*amount).await?;
            tracing::info!("Successfully withdrew {} ETH from the market", format_ether(*amount));
            Ok(())
        }
        AccountCommands::Balance { address } => {
            let client = config.build_client().await?;
            let addr = address.unwrap_or(client.boundless_market.caller());
            if addr == Address::ZERO {
                bail!("No address specified for balance query. Please provide an address or a private key.")
            }
            tracing::info!("Checking balance for address {}", addr);
            let balance = client.boundless_market.balance_of(addr).await?;
            tracing::info!("Balance for address {}: {} ETH", addr, format_ether(balance));
            Ok(())
        }
        AccountCommands::DepositCollateral { amount } => {
            let client = config.build_client_with_signer().await?;
            let (parsed_amount, formatted_amount, symbol) =
                parse_collateral_amount(&client, amount).await?;

            if !client.deployment.collateral_token_supports_permit() {
                tracing::info!("Approving {formatted_amount} {symbol} as collateral");
                client.boundless_market.approve_deposit_collateral(parsed_amount).await?;
                tracing::info!("Depositing {formatted_amount} {symbol} as collateral");
                match client.boundless_market.deposit_collateral(parsed_amount).await {
                    Ok(_) => {
                        tracing::info!(
                            "Successfully deposited {formatted_amount} {symbol} as collateral"
                        );
                        Ok(())
                    }
                    Err(e) => {
                        if e.to_string().contains("TRANSFER_FROM_FAILED") {
                            let addr = client.boundless_market.caller();
                            Err(anyhow!(
                                "Failed to deposit collateral: Ensure your address ({}) has funds on the {symbol} contract", addr
                            ))
                        } else {
                            Err(anyhow!("Failed to deposit collateral: {}", e))
                        }
                    }
                }
            } else {
                tracing::info!("Depositing {formatted_amount} {symbol} as collateral");
                match client
                    .boundless_market
                    .deposit_collateral_with_permit(parsed_amount, &client.signer.unwrap())
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Successfully deposited {formatted_amount} {symbol} as collateral"
                        );
                        Ok(())
                    }
                    Err(e) => {
                        if e.to_string().contains("TRANSFER_FROM_FAILED") {
                            let addr = client.boundless_market.caller();
                            Err(anyhow!(
                                "Failed to deposit collateral: Ensure your address ({}) has funds on the {symbol} contract", addr
                            ))
                        } else {
                            Err(anyhow!("Failed to deposit collateral: {}", e))
                        }
                    }
                }
            }
        }
        AccountCommands::WithdrawCollateral { amount } => {
            let client = config.build_client_with_signer().await?;
            let (parsed_amount, formatted_amount, symbol) =
                parse_collateral_amount(&client, amount).await?;
            tracing::info!("Withdrawing {formatted_amount} {symbol} from collateral");
            client.boundless_market.withdraw_collateral(parsed_amount).await?;
            tracing::info!("Successfully withdrew {formatted_amount} {symbol} from collateral");
            Ok(())
        }
        AccountCommands::CollateralBalance { address } => {
            let client = config.build_client().await?;
            let symbol = client.boundless_market.collateral_token_symbol().await?;
            let decimals = client.boundless_market.collateral_token_decimals().await?;
            let addr = address.unwrap_or(client.boundless_market.caller());
            if addr == Address::ZERO {
                bail!("No address specified for collateral balance query. Please provide an address or a private key.")
            }
            tracing::info!("Checking collateral balance for address {}", addr);
            let balance = client.boundless_market.balance_of_collateral(addr).await?;
            let balance = format_units(balance, decimals)
                .map_err(|e| anyhow!("Failed to format collateral balance: {}", e))?;
            tracing::info!("Collateral balance for address {}: {} {}", addr, balance, symbol);
            Ok(())
        }
    }
}

/// Handle request-related commands
async fn handle_request_command(cmd: &RequestCommands, config: &GlobalConfig) -> Result<()> {
    match cmd {
        RequestCommands::SubmitOffer(offer_args) => {
            let client = config
                .client_builder_with_signer()?
                .with_storage_provider_config(&offer_args.storage_config)?
                .build()
                .await
                .context("Failed to build Boundless Client")?;
            tracing::info!("Submitting new proof request with offer");
            submit_offer(client, offer_args).await
        }
        RequestCommands::Submit {
            yaml_request,
            wait,
            offchain,
            no_preflight,
            ref storage_config,
        } => {
            tracing::info!("Submitting proof request from YAML file");

            let client = config
                .client_builder_with_signer()?
                .with_storage_provider_config(storage_config)?
                .build()
                .await
                .context("Failed to build Boundless Client")?;
            submit_request(
                yaml_request,
                client,
                SubmitOptions { wait: *wait, offchain: *offchain, preflight: !*no_preflight },
            )
            .await
        }
        RequestCommands::Status { request_id, expires_at } => {
            let client = config.build_client().await?;
            tracing::info!("Checking status for request 0x{:x}", request_id);
            let status = client.boundless_market.get_status(*request_id, *expires_at).await?;
            tracing::info!("Request 0x{:x} status: {:?}", request_id, status);
            Ok(())
        }
        RequestCommands::GetProof { request_id } => {
            let client = config.build_client().await?;
            tracing::info!("Fetching proof for request 0x{:x}", request_id);
            let fulfillment = client.boundless_market.get_request_fulfillment(*request_id).await?;
            tracing::info!("Successfully retrieved proof for request 0x{:x}", request_id);
            tracing::info!(
                "Fulfillment Data: {} - Seal: {}",
                serde_json::to_string_pretty(&fulfillment.data()?)?,
                serde_json::to_string_pretty(&fulfillment.seal)?
            );
            Ok(())
        }
        RequestCommands::VerifyProof { request_id, image_id } => {
            let client = config.build_client().await?;
            tracing::info!("Verifying proof for request 0x{:x}", request_id);

            let verifier_address = client.deployment.verifier_router_address.context("no address provided for the verifier router; specify a verifier address with --verifier-address")?;
            let verifier = IRiscZeroVerifier::new(verifier_address, client.provider());
            let fulfillment = client.boundless_market.get_request_fulfillment(*request_id).await?;
            let fulfillment_data = fulfillment.data()?;
            let seal = fulfillment.seal;
            let (req, _) = client.boundless_market.get_submitted_request(*request_id, None).await?;

            let predicate = Predicate::try_from(req.requirements.predicate)?;

            match (&predicate, fulfillment_data.clone()) {
                (_, FulfillmentData::ImageIdAndJournal(image_id_from_data, journal)) => {
                    ensure!(
                        image_id_from_data == Digest::from(<[u8; 32]>::from(*image_id)),
                        "Image ID mismatch: expected {:?}, got {:?}",
                        image_id_from_data,
                        *image_id
                    );
                    let journal_digest =
                        <[u8; 32]>::from(Journal::new(journal.to_vec()).digest()).into();

                    verifier
                        .verify(seal, *image_id, journal_digest)
                        .call()
                        .await
                        .map_err(|_| anyhow::anyhow!("Verification failed"))?;
                }
                (_, _) => {
                    bail!(
                        "Verification failed due to invalid predicate {:?} or fulfillment data {:?}",
                        predicate,
                        fulfillment_data
                    )
                }
            }

            tracing::info!("Successfully verified proof for request 0x{:x}", request_id);
            Ok(())
        }
    }
}

/// Handle proving-related commands
async fn handle_proving_command(cmd: &ProvingCommands, config: &GlobalConfig) -> Result<()> {
    match cmd {
        ProvingCommands::Execute { request_path, request_id, request_digest, tx_hash } => {
            let client = config.build_client().await?;
            tracing::info!("Executing proof request");
            let request: ProofRequest = if let Some(file_path) = request_path {
                tracing::debug!("Loading request from file: {:?}", file_path);
                let file = File::open(file_path).context("failed to open request file")?;
                let reader = BufReader::new(file);
                serde_yaml::from_reader(reader).context("failed to parse request from YAML")?
            } else if let Some(request_id) = request_id {
                tracing::debug!("Loading request from blockchain: 0x{:x}", request_id);
                let (req, _signature) =
                    client.fetch_proof_request(*request_id, *tx_hash, *request_digest).await?;
                // TODO: We should check the signature here. If the signature is invalid, this
                // might lead to wasted time. Note though that if the signature is invalid it can
                // never be used to effect onchain state (e.g. locking or fulfilling).
                req
            } else {
                bail!("execute requires either a request file path or request ID")
            };

            let (image_id, session_info) = execute(&request).await?;
            let journal = session_info.journal.bytes;
            let predicate = Predicate::try_from(request.requirements.predicate.clone())?;

            let fulfillment_data =
                FulfillmentData::from_image_id_and_journal(image_id, journal.clone());

            if predicate.eval(&fulfillment_data).is_none() {
                tracing::error!("Predicate evaluation failed for request 0x{:x}", request.id);
                bail!("Predicate evaluation failed");
            }

            tracing::info!("Successfully executed request 0x{:x}", request.id);
            tracing::debug!("Journal: {:?}", journal);
            Ok(())
        }
        ProvingCommands::Fulfill {
            request_ids,
            request_digests,
            tx_hashes,
            withdraw,
            prover_config,
        } => {
            let client = config.build_client_with_signer().await?;
            if request_digests.is_some()
                && request_ids.len() != request_digests.as_ref().unwrap().len()
            {
                bail!("request_ids and request_digests must have the same length");
            }
            if tx_hashes.is_some() && request_ids.len() != tx_hashes.as_ref().unwrap().len() {
                bail!("request_ids and tx_hashes must have the same length");
            }

            let request_ids_string =
                request_ids.iter().map(|id| format!("0x{id:x}")).collect::<Vec<_>>().join(", ");
            tracing::info!("Fulfilling proof requests {}", request_ids_string);

            // Configure proving backend (defaults to bento like benchmark command)
            prover_config.configure_proving_backend_with_health_check().await?;

            let (_, market_url) = client.boundless_market.image_info().await?;
            tracing::debug!("Fetching Assessor program from {}", market_url);
            let assessor_program = fetch_url(&market_url).await?;
            let domain = client.boundless_market.eip712_domain().await?;

            let (_, set_builder_url) = client.set_verifier.image_info().await?;
            tracing::debug!("Fetching SetBuilder program from {}", set_builder_url);
            let set_builder_program = fetch_url(&set_builder_url).await?;

            let prover = DefaultProver::new(
                set_builder_program,
                assessor_program,
                client.boundless_market.caller(),
                domain,
            )?;

            let fetch_order_jobs = request_ids.iter().enumerate().map(|(i, request_id)| {
                let client = client.clone();
                let boundless_market = client.boundless_market.clone();
                async move {
                    let (req, sig) = client
                        .fetch_proof_request(
                            *request_id,
                            tx_hashes.as_ref().map(|tx_hashes| tx_hashes[i]),
                            request_digests.as_ref().map(|request_digests| request_digests[i]),
                        )
                        .await?;
                    tracing::debug!("Fetched order details: {req:?}");

                    if !req.is_smart_contract_signed() {
                        req.verify_signature(
                            &sig,
                            client.deployment.boundless_market_address,
                            boundless_market.get_chain_id().await?,
                        )?;
                    } else {
                        // TODO: Provide a way to check the EIP1271 auth.
                        tracing::debug!(
                            "Skipping authorization check on smart contract signed request 0x{:x}",
                            U256::from(req.id)
                        );
                    }
                    let is_locked = boundless_market.is_locked(*request_id).await?;
                    Ok::<_, anyhow::Error>((req, sig, is_locked))
                }
            });

            let results = futures::future::join_all(fetch_order_jobs).await;
            let mut orders = Vec::new();
            let mut unlocked_requests = Vec::new();

            for result in results {
                let (req, sig, is_locked) = result?;
                // If the request is not locked in, we need to "price" which checks the requirements
                // and assigns a price. Otherwise, we don't. This vec will be a singleton if not locked
                // and empty if the request is locked.
                if !is_locked {
                    unlocked_requests.push(UnlockedRequest::new(req.clone(), sig.clone()));
                }
                orders.push((req, sig));
            }

            let (fills, root_receipt, assessor_receipt) = prover.fulfill(&orders).await?;
            let order_fulfilled = OrderFulfilled::new(fills, root_receipt, assessor_receipt)?;
            let boundless_market = client.boundless_market.clone();

            let fulfillment_tx =
                FulfillmentTx::new(order_fulfilled.fills, order_fulfilled.assessorReceipt)
                    .with_submit_root(
                        client.deployment.set_verifier_address,
                        order_fulfilled.root,
                        order_fulfilled.seal,
                    )
                    .with_unlocked_requests(unlocked_requests)
                    .with_withdraw(*withdraw);
            match boundless_market.fulfill(fulfillment_tx).await {
                Ok(_) => {
                    tracing::info!("Successfully fulfilled requests {}", request_ids_string);
                    Ok(())
                }
                Err(e) => {
                    tracing::error!("Failed to fulfill requests {}: {}", request_ids_string, e);
                    bail!("Failed to fulfill request: {}", e)
                }
            }
        }
        ProvingCommands::Lock { request_id, request_digest, tx_hash } => {
            let client = config.build_client_with_signer().await?;
            tracing::info!("Locking proof request 0x{:x}", request_id);

            let (request, signature) =
                client.fetch_proof_request(*request_id, *tx_hash, *request_digest).await?;
            tracing::debug!("Fetched order details: {request:?}");

            // If the request is smart contract signed, the preflight of the lock request
            // transaction will revert, since it includes the ERC1271 signature check.
            if !request.is_smart_contract_signed() {
                request.verify_signature(
                    &signature,
                    client.deployment.boundless_market_address,
                    client.boundless_market.get_chain_id().await?,
                )?;
            }

            client.boundless_market.lock_request(&request, signature, None).await?;
            tracing::info!("Successfully locked request 0x{:x}", request_id);
            Ok(())
        }
        ProvingCommands::Benchmark { request_ids, prover_config } => {
            let client = config.build_client().await?;
            benchmark(client, request_ids, prover_config).await
        }
    }
}

/// Execute a proof request using the RISC Zero zkVM executor and measure performance
async fn benchmark<P: Provider + Clone + 'static>(
    client: Client<P, impl Any, impl Any, impl Any>,
    request_ids: &[U256],
    prover_config: &ProverConfig,
) -> Result<()> {
    tracing::info!("Starting benchmark for {} requests", request_ids.len());
    if request_ids.is_empty() {
        bail!("No request IDs provided");
    }

    if prover_config.use_default_prover {
        bail!("benchmark command does not support using the default prover");
    }
    prover_config.configure_proving_backend();
    let prover = BonsaiClient::from_env(risc0_zkvm::VERSION)?;

    // Track performance metrics across all runs
    let mut worst_khz = f64::MAX;
    let mut worst_time = 0.0;
    let mut worst_cycles = 0.0;
    let mut worst_request_id = U256::ZERO;

    // Check if we can connect to PostgreSQL using environment variables
    let pg_pool = match create_pg_pool().await {
        Ok(pool) => {
            tracing::info!("Successfully connected to PostgreSQL database");
            Some(pool)
        }
        Err(e) => {
            tracing::warn!("Failed to connect to PostgreSQL database: {}", e);
            None
        }
    };

    for (idx, request_id) in request_ids.iter().enumerate() {
        tracing::info!(
            "Benchmarking request {}/{}: 0x{:x}",
            idx + 1,
            request_ids.len(),
            request_id
        );

        let (request, _signature) = client.fetch_proof_request(*request_id, None, None).await?;
        // TODO: We should check the signature here. If the signature is invalid, this might lead
        // to wasted time on an invalid request. This is acceptable for now because the purpose of
        // this command is benchmarking.

        tracing::debug!("Fetched request 0x{:x}", request_id);
        tracing::debug!("Image URL: {}", request.imageUrl);

        // Fetch ELF and input
        tracing::debug!("Fetching ELF from {}", request.imageUrl);
        let elf = fetch_url(&request.imageUrl).await?;

        tracing::debug!("Processing input");
        let input = match request.input.inputType {
            RequestInputType::Inline => GuestEnv::decode(&request.input.data)?.stdin,
            RequestInputType::Url => {
                let input_url = std::str::from_utf8(&request.input.data)
                    .context("Input URL is not valid UTF-8")?;
                tracing::debug!("Fetching input from {}", input_url);
                GuestEnv::decode(&fetch_url(input_url).await?)?.stdin
            }
            _ => bail!("Unsupported input type"),
        };

        // Upload ELF
        let image_id = compute_image_id(&elf)?.to_string();
        prover.upload_img(&image_id, elf).await.unwrap();
        tracing::debug!("Uploaded ELF to {}", image_id);

        // Upload input
        let input_id =
            prover.upload_input(input).await.context("Failed to upload set-builder input")?;
        tracing::debug!("Uploaded input to {}", input_id);

        let assumptions = vec![];

        // Start timing
        let start_time = std::time::Instant::now();

        let proof_id =
            prover.create_session(image_id, input_id, assumptions.clone(), false).await?;
        tracing::debug!("Created session {}", proof_id.uuid);

        let (stats, elapsed_time) = loop {
            let status = proof_id.status(&prover).await?;

            match status.status.as_ref() {
                "RUNNING" => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    continue;
                }
                "SUCCEEDED" => {
                    let Some(stats) = status.stats else {
                        bail!("Bento failed to return proof stats in response");
                    };
                    break (stats, status.elapsed_time);
                }
                _ => {
                    let err_msg = status.error_msg.unwrap_or_default();
                    bail!("stark proving failed: {err_msg}");
                }
            }
        };

        // Try to get effective KHz from PostgreSQL if available
        let (total_cycles, elapsed_secs) = if let Some(ref pool) = pg_pool {
            let total_cycles_query = r#"
                SELECT (output->>'total_cycles')::FLOAT8
                FROM tasks
                WHERE task_id = 'init' AND job_id = $1::uuid
            "#;

            let elapsed_secs_query = r#"
                SELECT EXTRACT(EPOCH FROM (MAX(updated_at) - MIN(started_at)))::FLOAT8
                FROM tasks
                WHERE job_id = $1::uuid
            "#;

            let total_cycles: f64 =
                sqlx::query_scalar(total_cycles_query).bind(&proof_id.uuid).fetch_one(pool).await?;

            let elapsed_secs: f64 =
                sqlx::query_scalar(elapsed_secs_query).bind(&proof_id.uuid).fetch_one(pool).await?;

            (total_cycles, elapsed_secs)
        } else {
            // Calculate the hz based on the duration and total cycles as observed by the client
            tracing::debug!("No PostgreSQL data found for job, using client-side calculation.");
            let total_cycles: f64 = stats.total_cycles as f64;
            let elapsed_secs = start_time.elapsed().as_secs_f64();
            (total_cycles, elapsed_secs)
        };

        let khz = (total_cycles / 1000.0) / elapsed_secs;

        tracing::info!("KHz: {:.2} proved in {:.2}s", khz, elapsed_secs);

        if let Some(time) = elapsed_time {
            tracing::debug!("Server side time: {:?}", time);
        }

        // Track worst-case performance
        if khz < worst_khz {
            worst_khz = khz;
            worst_time = elapsed_secs;
            worst_cycles = total_cycles;
            worst_request_id = *request_id;
        }
    }

    if worst_cycles < 1_000_000.0 {
        tracing::warn!("Worst case performance proof is one with less than 1M cycles, \
            which might lead to a lower khz than expected. Benchmark using a larger proof if possible.");
    }

    // Report worst-case performance
    tracing::info!("Worst-case performance:");
    tracing::info!("  Request ID: 0x{:x}", worst_request_id);
    tracing::info!("  Performance: {:.2} KHz", worst_khz);
    tracing::info!("  Time: {:.2} seconds", worst_time);
    tracing::info!("  Cycles: {}", worst_cycles);

    println!("It is recommended to update this entry in broker.toml:");
    println!("peak_prove_khz = {:.0}\n", worst_khz.round());
    println!("Note: setting a lower value does not limit the proving speed, but will reduce the \
              total throughput of the orders locked by the broker. It is recommended to set a value \
              lower than this recommmendation, and increase it over time to increase capacity.");

    Ok(())
}

/// Create a PostgreSQL connection pool using environment variables
async fn create_pg_pool() -> Result<sqlx::PgPool, sqlx::Error> {
    let user = std::env::var("POSTGRES_USER").unwrap_or_else(|_| "worker".to_string());
    let password = std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "password".to_string());
    let db = std::env::var("POSTGRES_DB").unwrap_or_else(|_| "taskdb".to_string());
    let host = match std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "postgres".to_string()) {
        host if host != "postgres" => host,
        // Use local connection for postgres, as "postgres" not compatible with docker
        _ => "127.0.0.1".to_string(),
    };

    let port = std::env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());

    let connection_string = format!("postgres://{user}:{password}@{host}:{port}/{db}");

    sqlx::PgPool::connect(&connection_string).await
}

/// Submit an offer and create a proof request
async fn submit_offer(client: StandardClient, args: &SubmitOfferArgs) -> Result<()> {
    let request = client.new_request();

    // Resolve the program from command line arguments.
    let request = match (args.program.path.clone(), args.program.url.clone()) {
        (Some(path), None) => {
            if client.storage_provider.is_none() {
                bail!("A storage provider is required to upload programs.\nPlease provide a storage provider (see --help for options) or upload your program and set --program-url.")
            }
            let program: Cow<'static, [u8]> = std::fs::read(&path)
                .context(format!("Failed to read program file at {:?}", args.program))?
                .into();
            request.with_program(program)
        }
        (None, Some(url)) => request.with_program_url(url).map_err(|e| match e {}).unwrap(),
        _ => bail!("Exactly one of program path and program-url args must be provided"),
    };

    // Process input based on provided arguments
    let stdin: Vec<u8> = match (&args.input.input, &args.input.input_file) {
        (Some(input), None) => input.as_bytes().to_vec(),
        (None, Some(input_file)) => std::fs::read(input_file)
            .context(format!("Failed to read input file at {input_file:?}"))?,
        _ => bail!("Exactly one of input or input-file args must be provided"),
    };

    // Prepare the input environment
    let env = if args.encode_input {
        GuestEnv::builder().write(&stdin)?
    } else {
        GuestEnv::builder().write_slice(&stdin)
    };
    let request = request.with_env(env);

    // Configure callback if provided
    let mut requirements = RequirementParams::builder();
    if let Some(address) = args.requirements.callback_address {
        requirements.callback_address(address);
        if let Some(gas_limit) = args.requirements.callback_gas_limit {
            requirements.callback_gas_limit(gas_limit);
        }
    }
    match args.requirements.proof_type {
        // TODO(risc0-ethereum/#597): This needs to be kept up to date with releases of
        // risc0-ethereum. Add a Selector::inclusion_latest() function to risc0-ethereum and use it
        // here.
        ProofType::Inclusion => requirements.selector(Selector::set_inclusion_latest() as u32),
        ProofType::Groth16 => requirements.selector(Selector::groth16_latest() as u32),
        ProofType::Any => &mut requirements,
        ty => bail!("unsupported proof type provided in proof-type flag: {:?}", ty),
    };
    let request = request.with_requirements(requirements);

    let request = client.build_request(request).await.context("failed to build proof request")?;
    tracing::debug!("Request details: {}", serde_yaml::to_string(&request)?);

    // Submit the request
    let (request_id, expires_at) = if args.offchain {
        tracing::info!("Submitting request offchain");
        client.submit_request_offchain(&request).await?
    } else {
        tracing::info!("Submitting request onchain");
        client.submit_request_onchain(&request).await?
    };

    tracing::info!(
        "Submitted request 0x{request_id:x}, bidding starts at {}",
        convert_timestamp(request.offer.rampUpStart)
    );

    // Wait for fulfillment if requested
    if args.wait {
        tracing::info!("Waiting for request fulfillment...");
        let fulfillment = client
            .boundless_market
            .wait_for_request_fulfillment(request_id, Duration::from_secs(5), expires_at)
            .await?;
        let fulfillment_data = fulfillment.data()?;
        let seal = fulfillment.seal;

        tracing::info!("Request fulfilled!");
        tracing::info!(
            "Fulfillment Data: {} - Seal: {}",
            serde_json::to_string_pretty(&fulfillment_data)?,
            serde_json::to_string_pretty(&seal)?
        );
    }

    Ok(())
}

struct SubmitOptions {
    wait: bool,
    offchain: bool,
    preflight: bool,
}

/// Submit a proof request from a YAML file
async fn submit_request<P, S>(
    request_path: impl AsRef<Path>,
    client: Client<P, S>,
    opts: SubmitOptions,
) -> Result<()>
where
    P: Provider<Ethereum> + 'static + Clone,
    S: StorageProvider + Clone,
{
    // Read the YAML request file
    let file = File::open(request_path.as_ref())
        .context(format!("Failed to open request file at {:?}", request_path.as_ref()))?;
    let reader = BufReader::new(file);
    let mut request: ProofRequest =
        serde_yaml::from_reader(reader).context("Failed to parse request from YAML")?;

    // Fill in some of the request parameters, this command supports filling a few of the request
    // parameters that new need to updated on every reqeust. Namely, ID and bidding start.
    //
    // If set to 0, override the offer bidding_start field with the current timestamp + 30s
    if request.offer.rampUpStart == 0 {
        // Adding a delay to bidding start lets provers see and evaluate the request
        // before the price starts to ramp up
        request.offer = Offer { rampUpStart: now_timestamp() + 30, ..request.offer };
    }
    if request.id == U256::ZERO {
        request.id = client.boundless_market.request_id_from_rand().await?;
        tracing::info!("Assigned request ID {:x}", request.id);
    };

    // Run preflight check if enabled
    if opts.preflight {
        tracing::info!("Running request preflight check");
        let (image_id, session_info) = execute(&request).await?;
        let journal = session_info.journal.bytes;

        // Verify image ID
        if let Some(claim) = session_info.receipt_claim {
            ensure!(
                claim.pre.digest() == image_id,
                "Image ID mismatch: requirements ({}) do not match the given program ({})",
                image_id,
                claim.pre.digest(),
            );
        } else {
            tracing::debug!("Cannot check image ID; session info doesn't have receipt claim");
        }
        let predicate = Predicate::try_from(request.requirements.predicate.clone())?;

        ensure!(
            predicate.eval(&FulfillmentData::from_image_id_and_journal(image_id, journal.clone())).is_some(),
            "Preflight failed: Predicate evaluation failed. Journal: {}, Predicate type: {:?}, Predicate data: {}",
            hex::encode(&journal),
            request.requirements.predicate.predicateType,
            hex::encode(&request.requirements.predicate.data)
        );

        tracing::info!("Preflight check passed");
    } else {
        tracing::warn!("Skipping preflight check");
    }

    // Submit the request
    let (request_id, expires_at) = if opts.offchain {
        tracing::info!("Submitting request offchain");
        client.submit_request_offchain(&request).await?
    } else {
        tracing::info!("Submitting request onchain");
        client.submit_request_onchain(&request).await?
    };

    tracing::info!(
        "Submitted request 0x{request_id:x}, bidding starts at {}",
        convert_timestamp(request.offer.rampUpStart)
    );

    // Wait for fulfillment if requested
    if opts.wait {
        tracing::info!("Waiting for request fulfillment...");
        let fulfillment = client
            .wait_for_request_fulfillment(request_id, Duration::from_secs(5), expires_at)
            .await?;

        tracing::info!("Request fulfilled!");
        tracing::info!(
            "Fulfillment Data: {} - Seal: {}",
            serde_json::to_string_pretty(&fulfillment.data()?)?,
            serde_json::to_string_pretty(&fulfillment.seal)?
        );
    }

    Ok(())
}

/// Execute a proof request using the RISC Zero zkVM executor and returns the image id and session info
async fn execute(request: &ProofRequest) -> Result<(Digest, SessionInfo)> {
    tracing::info!("Fetching program from {}", request.imageUrl);
    let program = fetch_url(&request.imageUrl).await?;
    let image_id = compute_image_id(&program)?;
    tracing::info!("Processing input");
    let env = match request.input.inputType {
        RequestInputType::Inline => GuestEnv::decode(&request.input.data)?,
        RequestInputType::Url => {
            let input_url =
                std::str::from_utf8(&request.input.data).context("Input URL is not valid UTF-8")?;
            tracing::info!("Fetching input from {}", input_url);
            GuestEnv::decode(&fetch_url(input_url).await?)?
        }
        _ => bail!("Unsupported input type"),
    };

    tracing::info!("Executing program in zkVM");
    r0vm_is_installed()?;
    default_executor()
        .execute(env.try_into()?, &program)
        .map(|session_info| (image_id, session_info))
}

fn r0vm_is_installed() -> Result<()> {
    // Try to run the binary with the --version flag
    let result = std::process::Command::new("r0vm").arg("--version").output();

    match result {
        Ok(_) => Ok(()),
        Err(_) => Err(anyhow!("r0vm is not installed or could not be executed. Please check instructions at https://dev.risczero.com/api/zkvm/install")),
    }
}

// Get current timestamp with appropriate error handling
fn now_timestamp() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("Time went backwards").as_secs()
}

/// Handle config command
async fn handle_config_command(config: &GlobalConfig) -> Result<()> {
    tracing::info!("Displaying CLI configuration");
    println!("\n=== Boundless CLI Configuration ===\n");

    // Show configuration
    let rpc_url = config.require_rpc_url()?;
    println!("RPC URL: {rpc_url}");
    println!(
        "Wallet Address: {}",
        config
            .private_key
            .as_ref()
            .map(|sk| sk.address().to_string())
            .unwrap_or("[no wallet provided]".to_string())
    );
    if let Some(timeout) = config.tx_timeout {
        println!("Transaction Timeout: {} seconds", timeout.as_secs());
    } else {
        println!("Transaction Timeout: <not set>");
    }
    println!("Log Level: {:?}", config.log_level);
    if let Some(ref deployment) = config.deployment {
        println!("Using custom Boundless deployment");
        println!("Chain ID: {:?}", deployment.chain_id);
        println!("Boundless Market Address: {}", deployment.boundless_market_address);
        println!("Verifier Address: {:?}", deployment.verifier_router_address);
        println!("Set Verifier Address: {}", deployment.set_verifier_address);
        println!("Order Stream URL: {:?}", deployment.order_stream_url);
    }

    // Validate RPC connection
    println!("\n=== Environment Validation ===\n");
    print!("Testing RPC connection... ");
    let provider = ProviderBuilder::new().connect_http(rpc_url);

    let chain_id = match provider.get_chain_id().await {
        Ok(chain_id) => {
            println!("✅ Connected to chain ID: {chain_id}");
            chain_id
        }
        Err(e) => {
            println!("❌ Failed to connect: {e}");
            // Do not run remaining checks, which require an RPC connection.
            return Ok(());
        }
    };

    let Some(deployment) =
        config.deployment.clone().or_else(|| Deployment::from_chain_id(chain_id))
    else {
        println!("❌ No Boundless deployment config provided for unknown chain ID: {chain_id}");
        return Ok(());
    };

    // Check market contract
    print!("Testing Boundless Market contract... ");
    let boundless_market = BoundlessMarketService::new(
        deployment.boundless_market_address,
        provider.clone(),
        Address::ZERO,
    );

    let market_ok = match boundless_market.get_chain_id().await {
        Ok(_) => {
            println!("✅ Contract responds");
            true
        }
        Err(e) => {
            println!("❌ Contract error: {e}");
            false
        }
    };

    // Check set verifier contract
    print!("Testing Set Verifier contract... ");
    let set_verifier =
        SetVerifierService::new(deployment.set_verifier_address, provider.clone(), Address::ZERO);

    let (image_id, _) = match set_verifier.image_info().await {
        Ok(image_info) => {
            println!("✅ Contract responds");
            image_info
        }
        Err(e) => {
            println!("❌ Contract error: {e}");
            (B256::default(), String::default())
        }
    };

    // Create a transaction request with the call data
    if let Some(verifier_router_address) = deployment.verifier_router_address {
        let verifier_parameters =
            SetInclusionReceiptVerifierParameters { image_id: Digest::from_bytes(*image_id) };
        let selector: [u8; 4] = verifier_parameters.digest().as_bytes()[0..4].try_into()?;

        // Build the call data:
        // 1. Append the function selector for getVerifier(bytes4) ("3cadf449")
        // 2. Append the ABI encoding for the bytes4 parameter (padded to 32 bytes)
        let mut call_data = Vec::new();
        call_data.extend_from_slice(&hex::decode("3cadf449")?);
        call_data.extend_from_slice(&FixedBytes::from(selector).abi_encode());
        let tx = TransactionRequest {
            to: Some(TxKind::Call(verifier_router_address)),
            input: TransactionInput::new(call_data.into()),
            ..Default::default()
        };

        // Check verifier contract
        print!("Testing VerifierRouter contract... ");
        match provider.call(tx).await {
            Ok(_) => {
                println!("✅ Contract responds");
                true
            }
            Err(e) => {
                println!("❌ Contract error: {e}");
                false
            }
        };
    } else {
        // Verifier router is recommended, but not required for most operations.
        println!("⚠️ Verifier router address not configured");
    }

    println!(
        "\nEnvironment Setup: {}",
        if market_ok { "✅ Ready to use" } else { "❌ Issues detected" }
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use alloy::{
        node_bindings::{Anvil, AnvilInstance},
        primitives::{aliases::U96, utils::format_units, Bytes},
        providers::WalletProvider,
    };
    use boundless_market::{
        contracts::{
            hit_points::default_allowance, Predicate, RequestId, RequestInput, RequestStatus,
            Requirements,
        },
        selector::is_groth16_selector,
    };
    use boundless_test_utils::{
        guests::{ECHO_ID, ECHO_PATH},
        market::{create_test_ctx, deploy_mock_callback, get_mock_callback_count, TestCtx},
    };
    use order_stream::{run_from_parts, AppState, ConfigBuilder};
    use sqlx::PgPool;
    use tempfile::tempdir;
    use tokio::task::JoinHandle;
    use tracing::level_filters::LevelFilter;
    use tracing_test::traced_test;

    use super::*;

    // generate a test request
    fn generate_request(id: u32, addr: &Address) -> ProofRequest {
        ProofRequest::new(
            RequestId::new(*addr, id),
            Requirements::new(Predicate::prefix_match(ECHO_ID, Bytes::default())),
            format!("file://{ECHO_PATH}"),
            RequestInput::builder().write_slice(&[0x41, 0x41, 0x41, 0x41]).build_inline().unwrap(),
            Offer {
                minPrice: U256::from(20000000000000u64),
                maxPrice: U256::from(40000000000000u64),
                rampUpStart: now_timestamp(),
                timeout: 420,
                lockTimeout: 420,
                rampUpPeriod: 1,
                lockCollateral: U256::from(10),
            },
        )
    }

    enum AccountOwner {
        Customer,
        Prover,
    }

    /// Test setup helper that creates common test infrastructure
    async fn setup_test_env(
        owner: AccountOwner,
    ) -> (TestCtx<impl Provider + WalletProvider + Clone + 'static>, AnvilInstance, GlobalConfig)
    {
        let anvil = Anvil::new().spawn();

        let ctx = create_test_ctx(&anvil).await.unwrap();

        let private_key = match owner {
            AccountOwner::Customer => {
                ctx.prover_market
                    .deposit_collateral_with_permit(default_allowance(), &ctx.prover_signer)
                    .await
                    .unwrap();
                ctx.customer_signer.clone()
            }
            AccountOwner::Prover => ctx.prover_signer.clone(),
        };

        let config = GlobalConfig {
            rpc_url: Some(anvil.endpoint_url()),
            private_key: Some(private_key),
            deployment: Some(ctx.deployment.clone()),
            tx_timeout: None,
            log_level: LevelFilter::INFO,
        };

        (ctx, anvil, config)
    }

    async fn setup_test_env_with_order_stream(
        owner: AccountOwner,
        pool: PgPool,
    ) -> (
        TestCtx<impl Provider + WalletProvider + Clone + 'static>,
        AnvilInstance,
        GlobalConfig,
        JoinHandle<()>,
    ) {
        let (mut ctx, anvil, mut global_config) = setup_test_env(owner).await;

        // Create listener first
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
            .await
            .unwrap();
        let order_stream_address = listener.local_addr().unwrap();
        let order_stream_url = Url::parse(&format!("http://{order_stream_address}")).unwrap();
        let domain = order_stream_address.to_string();

        let config = ConfigBuilder::default()
            .rpc_url(anvil.endpoint_url())
            .market_address(ctx.deployment.boundless_market_address)
            .domain(domain)
            .build()
            .unwrap();

        // Start order stream server
        let order_stream = AppState::new(&config, Some(pool)).await.unwrap();
        let order_stream_clone = order_stream.clone();
        let order_stream_handle = tokio::spawn(async move {
            run_from_parts(order_stream_clone, listener).await.unwrap();
        });

        // Add the order_stream_url to the deployment config.
        ctx.deployment.order_stream_url = Some(order_stream_url.to_string().into());
        global_config.deployment = Some(ctx.deployment.clone());

        (ctx, anvil, global_config, order_stream_handle)
    }

    #[tokio::test]
    #[traced_test]
    async fn test_deposit_withdraw() {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let mut args = MainArgs {
            config,
            command: Command::Account(Box::new(AccountCommands::Deposit {
                amount: default_allowance(),
            })),
        };

        run(&args).await.unwrap();
        assert!(logs_contain(&format!(
            "Depositing {} ETH",
            format_units(default_allowance(), "ether").unwrap()
        )));
        assert!(logs_contain(&format!(
            "Successfully deposited {} ETH",
            format_units(default_allowance(), "ether").unwrap()
        )));

        let balance = ctx.prover_market.balance_of(ctx.customer_signer.address()).await.unwrap();
        assert_eq!(balance, default_allowance());

        args.command = Command::Account(Box::new(AccountCommands::Balance {
            address: Some(ctx.customer_signer.address()),
        }));
        run(&args).await.unwrap();
        assert!(logs_contain(&format!(
            "Checking balance for address {}",
            ctx.customer_signer.address()
        )));
        assert!(logs_contain(&format!(
            "Balance for address {}: {} ETH",
            ctx.customer_signer.address(),
            format_units(default_allowance(), "ether").unwrap()
        )));

        args.command =
            Command::Account(Box::new(AccountCommands::Withdraw { amount: default_allowance() }));

        run(&args).await.unwrap();
        assert!(logs_contain(&format!(
            "Withdrawing {} ETH",
            format_units(default_allowance(), "ether").unwrap()
        )));
        assert!(logs_contain(&format!(
            "Successfully withdrew {} ETH",
            format_units(default_allowance(), "ether").unwrap()
        )));

        let balance = ctx.prover_market.balance_of(ctx.customer_signer.address()).await.unwrap();
        assert_eq!(balance, U256::from(0));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_fail_deposit_withdraw() {
        let (_ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let amount = U256::from(10000000000000000000000_u128);
        let mut args = MainArgs {
            config,
            command: Command::Account(Box::new(AccountCommands::Deposit { amount })),
        };

        let err = run(&args).await.unwrap_err();
        assert!(err.to_string().contains("Insufficient funds"));

        args.command = Command::Account(Box::new(AccountCommands::Withdraw { amount }));

        let err = run(&args).await.unwrap_err();
        assert!(err.to_string().contains("InsufficientBalance"));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_deposit_withdraw_collateral() {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Prover).await;

        let mut args = MainArgs {
            config,
            command: Command::Account(Box::new(AccountCommands::DepositCollateral {
                amount: format_ether(default_allowance()),
            })),
        };

        run(&args).await.unwrap();
        assert!(logs_contain(&format!(
            "Depositing {} HP as collateral",
            format_ether(default_allowance())
        )));
        assert!(logs_contain(&format!(
            "Successfully deposited {} HP as collateral",
            format_ether(default_allowance())
        )));

        let balance =
            ctx.prover_market.balance_of_collateral(ctx.prover_signer.address()).await.unwrap();
        assert_eq!(balance, default_allowance());

        args.command = Command::Account(Box::new(AccountCommands::CollateralBalance {
            address: Some(ctx.prover_signer.address()),
        }));
        run(&args).await.unwrap();
        assert!(logs_contain(&format!(
            "Checking collateral balance for address {}",
            ctx.prover_signer.address()
        )));
        assert!(logs_contain(&format!(
            "Collateral balance for address {}: {} HP",
            ctx.prover_signer.address(),
            format_units(default_allowance(), "ether").unwrap()
        )));

        args.command = Command::Account(Box::new(AccountCommands::WithdrawCollateral {
            amount: format_ether(default_allowance()),
        }));

        run(&args).await.unwrap();
        assert!(logs_contain(&format!(
            "Withdrawing {} HP from collateral",
            format_ether(default_allowance())
        )));
        assert!(logs_contain(&format!(
            "Successfully withdrew {} HP from collateral",
            format_ether(default_allowance())
        )));

        let balance =
            ctx.prover_market.balance_of_collateral(ctx.prover_signer.address()).await.unwrap();
        assert_eq!(balance, U256::from(0));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_deposit_collateral_amount_below_denom_min() -> Result<()> {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        // Use amount below denom min
        let amount = "0.00000000000000000000000001".to_string();
        let args = MainArgs {
            config,
            command: Command::Account(Box::new(AccountCommands::DepositCollateral {
                amount: amount.clone(),
            })),
        };

        // Sanity check to make sure that the amount is below the denom min
        let decimals = ctx.customer_market.collateral_token_decimals().await?;
        let parsed_amount: U256 = parse_units(&amount, decimals).unwrap().into();
        assert_eq!(parsed_amount, U256::from(0));

        let err = run(&args).await.unwrap_err();
        assert!(err.to_string().contains("Amount is below the denomination minimum"));

        Ok(())
    }

    #[tokio::test]
    #[traced_test]
    async fn test_fail_deposit_withdraw_collateral() {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let mut args = MainArgs {
            config,
            command: Command::Account(Box::new(AccountCommands::DepositCollateral {
                amount: format_ether(default_allowance()),
            })),
        };

        let err = run(&args).await.unwrap_err();
        assert!(err.to_string().contains(&format!(
            "Failed to deposit collateral: Ensure your address ({}) has funds on the HP contract",
            ctx.customer_signer.address()
        )));

        args.command = Command::Account(Box::new(AccountCommands::WithdrawCollateral {
            amount: format_ether(default_allowance()),
        }));

        let err = run(&args).await.unwrap_err();
        assert!(err.to_string().contains("InsufficientBalance"));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_submit_request_onchain() {
        let (_ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        // Submit a request onchain
        let args = MainArgs {
            config,
            command: Command::Request(Box::new(RequestCommands::Submit {
                storage_config: Box::new(StorageProviderConfig::dev_mode()),
                yaml_request: "../../request.yaml".to_string().into(),
                wait: false,
                offchain: false,
                no_preflight: false,
            })),
        };
        run(&args).await.unwrap();
        assert!(logs_contain("Submitting request onchain"));
        assert!(logs_contain("Submitted request"));
    }

    #[sqlx::test]
    #[traced_test]
    async fn test_submit_request_offchain(pool: PgPool) {
        let (ctx, _anvil, config, order_stream_handle) =
            setup_test_env_with_order_stream(AccountOwner::Customer, pool).await;

        // Deposit funds into the market
        ctx.customer_market.deposit(parse_ether("1").unwrap()).await.unwrap();

        // Submit a request offchain
        let args = MainArgs {
            config,
            command: Command::Request(Box::new(RequestCommands::Submit {
                storage_config: Box::new(StorageProviderConfig::dev_mode()),
                yaml_request: "../../request.yaml".to_string().into(),
                wait: false,
                offchain: true,
                no_preflight: true,
            })),
        };
        run(&args).await.unwrap();
        assert!(logs_contain("Submitting request offchain"));
        assert!(logs_contain("Submitted request"));

        // Clean up
        order_stream_handle.abort();
    }

    #[tokio::test]
    #[traced_test]
    async fn test_submit_offer_onchain() {
        let (_ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        // Submit a request onchain
        let args = MainArgs {
            config,
            command: Command::Request(Box::new(RequestCommands::SubmitOffer(Box::new(
                SubmitOfferArgs {
                    storage_config: StorageProviderConfig::dev_mode(),
                    id: None,
                    wait: false,
                    offchain: false,
                    encode_input: false,
                    input: SubmitOfferInput {
                        input: Some(hex::encode([0x41, 0x41, 0x41, 0x41])),
                        input_file: None,
                    },
                    program: SubmitOfferProgram { path: Some(PathBuf::from(ECHO_PATH)), url: None },
                    requirements: SubmitOfferRequirements {
                        callback_address: None,
                        callback_gas_limit: None,
                        proof_type: ProofType::Any,
                    },
                    offer_params: OfferParams::default(),
                },
            )))),
        };
        run(&args).await.unwrap();
        assert!(logs_contain("Submitting request onchain"));
        assert!(logs_contain("Submitted request"));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_request_status_onchain() {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let request = generate_request(
            ctx.customer_market.index_from_nonce().await.unwrap(),
            &ctx.customer_signer.address(),
        );

        // Deposit funds into the market
        ctx.customer_market.deposit(parse_ether("1").unwrap()).await.unwrap();

        // Submit the request onchain
        ctx.customer_market.submit_request(&request, &ctx.customer_signer).await.unwrap();

        // Create a new args struct to test the Status command
        let status_args = MainArgs {
            config,
            command: Command::Request(Box::new(RequestCommands::Status {
                request_id: request.id,
                expires_at: None,
            })),
        };

        run(&status_args).await.unwrap();

        assert!(logs_contain(&format!("Request 0x{:x} status: Unknown", request.id)));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_slash() {
        let (ctx, anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let mut request = generate_request(
            ctx.customer_market.index_from_nonce().await.unwrap(),
            &ctx.customer_signer.address(),
        );
        request.offer.timeout = 50;
        request.offer.lockTimeout = 50;

        // Deposit funds into the market
        ctx.customer_market.deposit(parse_ether("1").unwrap()).await.unwrap();

        // Submit the request onchain
        ctx.customer_market.submit_request(&request, &ctx.customer_signer).await.unwrap();

        let client_sig = request
            .sign_request(
                &ctx.customer_signer,
                ctx.deployment.boundless_market_address,
                anvil.chain_id(),
            )
            .await
            .unwrap();

        // Lock the request
        ctx.prover_market
            .lock_request(&request, client_sig.as_bytes().to_vec(), None)
            .await
            .unwrap();

        // Create a new args struct to test the Status command
        let status_args = MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::Status {
                request_id: request.id,
                expires_at: None,
            })),
        };
        run(&status_args).await.unwrap();
        assert!(logs_contain(&format!("Request 0x{:x} status: Locked", request.id)));

        loop {
            // Wait for the timeout to expire
            tokio::time::sleep(Duration::from_secs(1)).await;
            let status = ctx
                .customer_market
                .get_status(request.id, Some(request.expires_at()))
                .await
                .unwrap();
            if status == RequestStatus::Expired {
                break;
            }
        }

        // test the Slash command
        run(&MainArgs {
            config,
            command: Command::Ops(Box::new(OpsCommands::Slash { request_id: request.id })),
        })
        .await
        .unwrap();
        assert!(logs_contain(&format!(
            "Successfully slashed prover for request 0x{:x}",
            request.id
        )));
    }

    #[tokio::test]
    #[traced_test]
    #[ignore = "Generates a proof. Slow without RISC0_DEV_MODE=1"]
    async fn test_proving_onchain() {
        let (ctx, anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let request = generate_request(
            ctx.customer_market.index_from_nonce().await.unwrap(),
            &ctx.customer_signer.address(),
        );

        let request_id = request.id;

        // Dump the request to a tmp file; tmp is deleted on drop.
        let tmp = tempdir().unwrap();
        let request_path = tmp.path().join("request.yaml");
        let request_file = File::create(&request_path).unwrap();
        serde_yaml::to_writer(request_file, &request).unwrap();

        // send the request onchain
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::Submit {
                storage_config: Box::new(StorageProviderConfig::dev_mode()),
                yaml_request: request_path,
                wait: false,
                offchain: false,
                no_preflight: true,
            })),
        })
        .await
        .unwrap();

        // test the Execute command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Proving(Box::new(ProvingCommands::Execute {
                request_path: None,
                request_id: Some(request_id),
                request_digest: None,
                tx_hash: None,
            })),
        })
        .await
        .unwrap();

        assert!(logs_contain(&format!("Successfully executed request 0x{:x}", request.id)));

        let prover_config = GlobalConfig {
            rpc_url: Some(anvil.endpoint_url()),
            private_key: Some(ctx.prover_signer.clone()),
            deployment: Some(ctx.deployment),
            tx_timeout: None,
            log_level: LevelFilter::INFO,
        };

        // test the Lock command
        run(&MainArgs {
            config: prover_config,
            command: Command::Proving(Box::new(ProvingCommands::Lock {
                request_id,
                request_digest: None,
                tx_hash: None,
            })),
        })
        .await
        .unwrap();
        assert!(logs_contain(&format!("Successfully locked request 0x{:x}", request.id)));

        // test the Status command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::Status {
                request_id,
                expires_at: None,
            })),
        })
        .await
        .unwrap();
        assert!(logs_contain(&format!("Request 0x{:x} status: Locked", request.id)));

        // test the Fulfill command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Proving(Box::new(ProvingCommands::Fulfill {
                request_ids: vec![request_id],
                request_digests: None,
                tx_hashes: None,
                withdraw: false,
                prover_config: ProverConfig {
                    bento_api_key: None,
                    bento_api_url: "".to_string(),
                    use_default_prover: true,
                    skip_health_check: true,
                },
            })),
        })
        .await
        .unwrap();

        assert!(logs_contain(&format!("Successfully fulfilled requests 0x{:x}", request.id)));

        // test the Status command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::Status {
                request_id,
                expires_at: None,
            })),
        })
        .await
        .unwrap();
        assert!(logs_contain(&format!("Request 0x{:x} status: Fulfilled", request.id)));

        // test the GetProof command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::GetProof { request_id })),
        })
        .await
        .unwrap();
        assert!(logs_contain(&format!(
            "Successfully retrieved proof for request 0x{:x}",
            request.id
        )));

        let predicate = Predicate::try_from(request.requirements.predicate.clone()).unwrap();

        // test the Verify command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::VerifyProof {
                request_id,
                image_id: <[u8; 32]>::from(predicate.image_id().unwrap()).into(),
            })),
        })
        .await
        .unwrap();
        assert!(logs_contain(&format!(
            "Successfully verified proof for request 0x{:x}",
            request.id
        )));
    }

    #[tokio::test]
    #[traced_test]
    #[ignore = "Generates a proof. Slow without RISC0_DEV_MODE=1"]
    async fn test_proving_multiple_requests() {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let mut request_ids = Vec::new();
        for _ in 0..3 {
            let request = generate_request(
                ctx.customer_market.index_from_nonce().await.unwrap(),
                &ctx.customer_signer.address(),
            );

            ctx.customer_market.submit_request(&request, &ctx.customer_signer).await.unwrap();
            request_ids.push(request.id);
        }

        // test the Fulfill command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Proving(Box::new(ProvingCommands::Fulfill {
                request_ids: request_ids.clone(),
                request_digests: None,
                tx_hashes: None,
                withdraw: false,
                prover_config: ProverConfig {
                    bento_api_key: None,
                    bento_api_url: "".to_string(),
                    use_default_prover: true,
                    skip_health_check: true,
                },
            })),
        })
        .await
        .unwrap();

        let request_ids_str =
            request_ids.iter().map(|id| format!("0x{id:x}")).collect::<Vec<_>>().join(", ");
        assert!(logs_contain(&format!("Successfully fulfilled requests {request_ids_str}")));

        for request_id in request_ids {
            // test the Status command
            run(&MainArgs {
                config: config.clone(),
                command: Command::Request(Box::new(RequestCommands::Status {
                    request_id,
                    expires_at: None,
                })),
            })
            .await
            .unwrap();
            assert!(logs_contain(&format!("Request 0x{request_id:x} status: Fulfilled")));
        }
    }

    #[tokio::test]
    #[traced_test]
    #[ignore = "Generates a proof. Slow without RISC0_DEV_MODE=1"]
    async fn test_callback() {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let mut request = generate_request(
            ctx.customer_market.index_from_nonce().await.unwrap(),
            &ctx.customer_signer.address(),
        );

        // Deploy MockCallback contract
        let callback_address = deploy_mock_callback(
            &ctx.prover_provider,
            ctx.deployment.verifier_router_address.unwrap(),
            ctx.deployment.boundless_market_address,
            ECHO_ID,
            U256::ZERO,
        )
        .await
        .unwrap();

        // Update the request with the callback address
        request.requirements.callback.addr = callback_address;
        request.requirements.callback.gasLimit = U96::from(100000);

        // Dump the request to a tmp file; tmp is deleted on drop.
        let tmp = tempdir().unwrap();
        let request_path = tmp.path().join("request.yaml");
        let request_file = File::create(&request_path).unwrap();
        serde_yaml::to_writer(request_file, &request).unwrap();

        // send the request onchain
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::Submit {
                storage_config: Box::new(StorageProviderConfig::dev_mode()),
                yaml_request: request_path,
                wait: false,
                offchain: false,
                no_preflight: true,
            })),
        })
        .await
        .unwrap();

        // fulfill the request
        run(&MainArgs {
            config,
            command: Command::Proving(Box::new(ProvingCommands::Fulfill {
                request_ids: vec![request.id],
                request_digests: None,
                tx_hashes: None,
                withdraw: false,
                prover_config: ProverConfig {
                    bento_api_key: None,
                    bento_api_url: "".to_string(),
                    use_default_prover: true,
                    skip_health_check: true,
                },
            })),
        })
        .await
        .unwrap();

        // check the callback was called
        let count =
            get_mock_callback_count(&ctx.customer_provider, callback_address).await.unwrap();
        assert!(count == U256::from(1));
    }

    #[tokio::test]
    #[traced_test]
    #[ignore = "Generates a proof. Slow without RISC0_DEV_MODE=1"]
    async fn test_selector() {
        let (ctx, _anvil, config) = setup_test_env(AccountOwner::Customer).await;

        let mut request = generate_request(
            ctx.customer_market.index_from_nonce().await.unwrap(),
            &ctx.customer_signer.address(),
        );

        // Explicitly set the selector to a compatible value for the test
        // In dev mode, instead of Groth16, use FakeReceipt
        request.requirements.selector = FixedBytes::from(Selector::FakeReceipt as u32);

        // Dump the request to a tmp file; tmp is deleted on drop.
        let tmp = tempdir().unwrap();
        let request_path = tmp.path().join("request.yaml");
        let request_file = File::create(&request_path).unwrap();
        serde_yaml::to_writer(request_file, &request).unwrap();

        // send the request onchain
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::Submit {
                storage_config: Box::new(StorageProviderConfig::dev_mode()),
                yaml_request: request_path,
                wait: false,
                offchain: false,
                no_preflight: true,
            })),
        })
        .await
        .unwrap();

        // fulfill the request
        run(&MainArgs {
            config,
            command: Command::Proving(Box::new(ProvingCommands::Fulfill {
                request_ids: vec![request.id],
                request_digests: None,
                tx_hashes: None,
                withdraw: false,
                prover_config: ProverConfig {
                    bento_api_key: None,
                    bento_api_url: "".to_string(),
                    use_default_prover: true,
                    skip_health_check: true,
                },
            })),
        })
        .await
        .unwrap();

        // check the seal is aggregated
        let fulfillment = ctx.customer_market.get_request_fulfillment(request.id).await.unwrap();
        let seal = fulfillment.seal;
        let selector: FixedBytes<4> = seal[0..4].try_into().unwrap();
        assert!(is_groth16_selector(selector))
    }

    #[sqlx::test]
    #[traced_test]
    #[ignore = "Generates a proof. Slow without RISC0_DEV_MODE=1"]
    async fn test_proving_offchain(pool: PgPool) {
        let (ctx, anvil, config, order_stream_handle) =
            setup_test_env_with_order_stream(AccountOwner::Customer, pool).await;

        // Deposit funds into the market
        ctx.customer_market.deposit(parse_ether("1").unwrap()).await.unwrap();

        let request = generate_request(
            ctx.customer_market.index_from_nonce().await.unwrap(),
            &ctx.customer_signer.address(),
        );

        let request_id = request.id;

        // Dump the request to a tmp file; tmp is deleted on drop.
        let tmp = tempdir().unwrap();
        let request_path = tmp.path().join("request.yaml");
        let request_file = File::create(&request_path).unwrap();
        serde_yaml::to_writer(request_file, &request).unwrap();

        // send the request offchain
        run(&MainArgs {
            config: config.clone(),
            command: Command::Request(Box::new(RequestCommands::Submit {
                storage_config: Box::new(StorageProviderConfig::dev_mode()),
                yaml_request: request_path,
                wait: false,
                offchain: true,
                no_preflight: true,
            })),
        })
        .await
        .unwrap();

        // test the Execute command
        run(&MainArgs {
            config: config.clone(),
            command: Command::Proving(Box::new(ProvingCommands::Execute {
                request_path: None,
                request_id: Some(request_id),
                request_digest: None,
                tx_hash: None,
            })),
        })
        .await
        .unwrap();

        assert!(logs_contain(&format!("Successfully executed request 0x{:x}", request.id)));

        let prover_config = GlobalConfig {
            rpc_url: Some(anvil.endpoint_url()),
            private_key: Some(ctx.prover_signer.clone()),
            deployment: Some(ctx.deployment),
            tx_timeout: None,
            log_level: LevelFilter::INFO,
        };

        // test the Lock command
        run(&MainArgs {
            config: prover_config,
            command: Command::Proving(Box::new(ProvingCommands::Lock {
                request_id,
                request_digest: None,
                tx_hash: None,
            })),
        })
        .await
        .unwrap();
        assert!(logs_contain(&format!("Successfully locked request 0x{:x}", request.id)));

        // test the Fulfill command
        run(&MainArgs {
            config,
            command: Command::Proving(Box::new(ProvingCommands::Fulfill {
                request_ids: vec![request_id],
                request_digests: None,
                tx_hashes: None,
                withdraw: true,
                prover_config: ProverConfig {
                    bento_api_key: None,
                    bento_api_url: "".to_string(),
                    use_default_prover: true,
                    skip_health_check: true,
                },
            })),
        })
        .await
        .unwrap();

        assert!(logs_contain(&format!("Successfully fulfilled requests 0x{:x}", request.id)));

        // test the automated withdraw
        let balance = ctx.prover_market.balance_of(ctx.prover_signer.address()).await.unwrap();
        assert_eq!(balance, U256::from(0));

        // Clean up
        order_stream_handle.abort();
    }
}
