use clap::{AppSettings, Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use tonic::transport::{Channel, ClientTlsConfig};

use grpc_jobruntime::job_runtime_client::JobRuntimeClient;
use grpc_jobruntime::{
    JobLogsRequest, JobResourceLimits, JobStartRequest, JobStatusRequest, JobStopRequest,
};

pub mod grpc_jobruntime {
    tonic::include_proto!("jobruntime");
}

mod tls;

#[derive(Parser)]
#[clap(name = "jobclient")]
struct Cli {
    #[clap(long)]
    key: String,
    #[clap(long)]
    cert: PathBuf,
    #[clap(long)]
    ca_cert: PathBuf,
    #[clap(long)]
    addr: String,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[clap(setting(AppSettings::ArgRequiredElseHelp))]
    Start {
        #[clap(required = true)]
        args: Vec<String>,

        #[clap(flatten)]
        limits: ResourceLimits,
    },
    FetchLogs {
        uuid: String,
    },
    Stop {
        uuid: String,
    },
    Status {
        uuid: String,
    },
}

#[derive(Parser)]
struct ResourceLimits {
    #[clap(name = "memory_high", long)]
    memory_high: Option<u64>,
    #[clap(name = "memory_max", long)]
    memory_max: Option<u64>,
    #[clap(name = "cpu_weight", long)]
    cpu_weight: Option<u32>,
    #[clap(name = "cpu_max", long)]
    cpu_max: Option<u32>,
    #[clap(name = "io_weight", long)]
    io_weight: Option<u32>,
}

impl From<ResourceLimits> for Option<JobResourceLimits> {
    fn from(limits: ResourceLimits) -> Self {
        if limits.memory_high.is_none()
            && limits.memory_max.is_none()
            && limits.cpu_weight.is_none()
            && limits.cpu_max.is_none()
            && limits.io_weight.is_none()
        {
            return None;
        };

        Some(JobResourceLimits {
            memory_high: limits.memory_high.unwrap_or_default(),
            memory_max: limits.memory_max.unwrap_or_default(),
            cpu_weight: limits.cpu_weight.unwrap_or_default(),
            cpu_max: limits.cpu_max.unwrap_or_default(),
            io_weight: limits.io_weight.unwrap_or_default(),
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    // skip parsing for now, assume valid address
    let cert = fs::read(args.cert)?;
    let key = fs::read(args.key)?;
    let ca_cert = fs::read(args.ca_cert)?;

    let client_config = tls::prepare_client_config(cert, key, ca_cert)?;
    let client_tls_config = ClientTlsConfig::new().rustls_client_config(client_config);

    let channel = Channel::from_shared(args.addr)?
        .tls_config(client_tls_config)?
        .connect()
        .await?;

    let mut client = JobRuntimeClient::new(channel);

    match args.command {
        Commands::Start { args, limits } => {
            let request = JobStartRequest {
                args,
                limits: limits.into(),
            };
            let result = client.start_job(request).await?;
            println!("{}", result.get_ref().uuid);
        }
        Commands::FetchLogs { uuid } => {
            let request = JobLogsRequest { uuid };
            let result = client.fetch_job_logs(request).await?;

            let mut stream = result.into_inner();
            while let Some(res) = stream.message().await? {
                print!("{}", String::from_utf8_lossy(&res.data));
            }
        }
        Commands::Stop { uuid } => {
            let request = JobStopRequest { uuid };
            client.stop_job(request).await?;
        }
        Commands::Status { uuid } => {
            let request = JobStatusRequest { uuid };
            let result = client.fetch_job_status(request).await?;
            let job_status = result.get_ref();
            println!("{} - {:?}", job_status.uuid, job_status.status);
        }
    }

    Ok(())
}
