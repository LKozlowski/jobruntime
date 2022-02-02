use tokio_rustls::rustls::{ciphersuite, Certificate, ClientConfig, PrivateKey, ProtocolVersion};

pub fn prepare_client_config(
    cert: Vec<u8>,
    key: Vec<u8>,
    ca: Vec<u8>,
) -> anyhow::Result<ClientConfig> {
    let mut config = ClientConfig::with_ciphersuites(&[
        &ciphersuite::TLS13_AES_128_GCM_SHA256,
        &ciphersuite::TLS13_AES_256_GCM_SHA384,
        &ciphersuite::TLS13_CHACHA20_POLY1305_SHA256,
    ]);
    config.versions = vec![ProtocolVersion::TLSv1_3];
    config
        .root_store
        .add(&Certificate(ca))
        .expect("invalid ca certificate");
    config.set_single_client_cert(vec![Certificate(cert)], PrivateKey(key))?;
    config.set_protocols(&[b"h2".to_vec()]);

    Ok(config)
}
