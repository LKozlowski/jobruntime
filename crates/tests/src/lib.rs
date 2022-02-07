#[cfg(test)]
mod test {
    use jobdaemon::grpc::grpc_jobruntime::job_runtime_server::JobRuntimeServer;
    use jobdaemon::grpc::MyJobRuntime;
    use jobdaemon::tls::prepare_server_config;
    use runtime::JobRuntime;
    use tonic::transport::{Channel, ClientTlsConfig, Server, ServerTlsConfig};

    #[tokio::test]
    async fn test_auth_with_different_ca_certs() -> anyhow::Result<()> {
        let server_cert = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/certs/ca1/server.cert.der"
        ))
        .unwrap();
        let server_key = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/certs/ca1/server.key.der"
        ))
        .unwrap();
        let server_ca_cert = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/certs/ca1/ca.cert.der"
        ))
        .unwrap();
        let admin_cert = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/certs/ca2/admin.cert.der"
        ))
        .unwrap();
        let admin_key = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/certs/ca2/admin.key.der"
        ))
        .unwrap();
        let admin_ca_cert = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/certs/ca2/ca.cert.der"
        ))
        .unwrap();

        let (_, cmd_tx) = JobRuntime::new();
        let runtime = MyJobRuntime::new(cmd_tx);
        let mut tls_config = ServerTlsConfig::new();

        tls_config.rustls_server_config(
            prepare_server_config(server_cert, server_key, server_ca_cert).unwrap(),
        );

        tokio::spawn(async {
            Server::builder()
                .tls_config(tls_config)
                .unwrap()
                .add_service(JobRuntimeServer::with_interceptor(
                    runtime,
                    jobdaemon::tls::intercept_extract_username_from_certificate,
                ))
                .serve("127.0.0.1:50050".parse().unwrap())
                .await
        });

        let client_config =
            jobclient::tls::prepare_client_config(admin_cert, admin_key, admin_ca_cert)?;
        let client_tls_config = ClientTlsConfig::new().rustls_client_config(client_config);

        let channel = Channel::from_shared("https://localhost:50050")?
            .tls_config(client_tls_config)?
            .connect()
            .await;

        assert_eq!(channel.is_err(), true);
        Ok(())
    }
}
