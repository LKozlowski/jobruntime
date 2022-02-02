use crate::grpc::Username;
use tokio_rustls::rustls::{
    ciphersuite, AllowAnyAuthenticatedClient, Certificate, PrivateKey, RootCertStore, ServerConfig,
};
use tonic::{Request, Status};
use x509_parser::certificate::X509Certificate;

pub struct UsernameExtension {
    pub username: Username,
}

pub fn prepare_server_config(
    cert: Vec<u8>,
    key: Vec<u8>,
    ca: Vec<u8>,
) -> anyhow::Result<ServerConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(&Certificate(ca)).expect("invalid ca certificate");

    let mut config = ServerConfig::with_ciphersuites(
        AllowAnyAuthenticatedClient::new(roots),
        &[
            &ciphersuite::TLS13_AES_128_GCM_SHA256,
            &ciphersuite::TLS13_AES_256_GCM_SHA384,
            &ciphersuite::TLS13_CHACHA20_POLY1305_SHA256,
        ],
    );
    config.set_single_cert(vec![Certificate(cert)], PrivateKey(key))?;
    config.set_protocols(&[b"h2".to_vec()]);
    Ok(config)
}

pub fn extract_name_from_certificate(cert: X509Certificate<'_>) -> anyhow::Result<Username> {
    // there should be better validation, but for now I'll only check the first CommonName from the certificate
    let name = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok());

    match name {
        Some(name) => Ok(String::from(name)),
        None => anyhow::bail!("invalid CommonName in certificate"),
    }
}

pub fn intercept_extract_username_from_certificate(
    mut request: Request<()>,
) -> Result<Request<()>, Status> {
    // it will only fail if we didn't enable tls feature
    let certs = request.peer_certs().unwrap();
    let cert = certs
        .iter()
        .next()
        .ok_or_else(|| Status::unauthenticated("no certificate found"))?;
    let (_, x509_cert) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|err| Status::unauthenticated(format!("failed to parse certificate: {}", err)))?;
    request.extensions_mut().insert(UsernameExtension {
        username: extract_name_from_certificate(x509_cert).map_err(|err| {
            Status::unauthenticated(format!("failed to extract username: {}", err))
        })?,
    });
    Ok(request)
}

#[cfg(test)]
mod test {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    // I'm testing only for happy path here, but there should also be a lot more tests for invalid certs, invalid common names, etc
    #[test]
    fn extract_name_from_certificate_extracts_valid_name() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests/certs/admin.cert.der");

        println!("{:?}", path);
        let admin_cert = fs::read(path).unwrap();
        let (_, cert) = x509_parser::parse_x509_certificate(&admin_cert).unwrap();
        assert_eq!(extract_name_from_certificate(cert).unwrap(), "admin");
    }
}
