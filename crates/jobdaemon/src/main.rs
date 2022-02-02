pub mod grpc;
pub mod tls;

use clap::Parser;
use grpc::grpc_jobruntime::job_runtime_server::JobRuntimeServer;
use grpc::MyJobRuntime;
use runtime::JobRuntime;
use tonic::transport::{Server, ServerTlsConfig};

#[derive(Parser)]
#[clap(name = "jobdaemon")]
struct Cli {
    #[clap(long)]
    key: String,
    #[clap(long)]
    cert: String,
    #[clap(long)]
    ca_cert: String,
    #[clap(long)]
    addr: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    let (rt, cmd_tx) = JobRuntime::new();
    let rt = rt.enable_cgroups()?;

    let runtime = MyJobRuntime::new(cmd_tx);
    tokio::spawn(async {
        rt.start().await;
    });

    let server_key = std::fs::read(args.key)?;
    let server_cert = std::fs::read(args.cert)?;
    let ca_cert = std::fs::read(args.ca_cert)?;

    let mut tls_config = ServerTlsConfig::new();
    tls_config.rustls_server_config(tls::prepare_server_config(
        server_cert,
        server_key,
        ca_cert,
    )?);

    println!("starting server at: {}", args.addr);

    Server::builder()
        .tls_config(tls_config)?
        .add_service(JobRuntimeServer::with_interceptor(
            runtime,
            tls::intercept_extract_username_from_certificate,
        ))
        .serve(args.addr.parse().unwrap())
        .await?;

    Ok(())
}
