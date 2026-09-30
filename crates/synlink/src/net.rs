//! QUIC-узел: одна конечная точка и для входящих, и для исходящих.
//!
//! TLS 1.3 с взаимной проверкой: у каждого устройства самоподписанный
//! сертификат, цепочки не проверяются, но подпись рукопожатия — да (значит,
//! собеседник владеет ключом). Доверие — по отпечатку сертификата, его
//! сверяет `session` (закреплён при спаривании).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

use crate::identity::Identity;
use crate::proto::ALPN;

#[derive(Debug)]
struct PinnedLater(Arc<CryptoProvider>);

impl ServerCertVerifier for PinnedLater {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl ClientCertVerifier for PinnedLater {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn transport() -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    t.keep_alive_interval(Some(Duration::from_secs(3)));
    t.max_idle_timeout(Some(Duration::from_secs(12).try_into().unwrap()));
    t.max_concurrent_bidi_streams(1024u32.into());
    t.max_concurrent_uni_streams(64u32.into());
    // Окна побольше: кадры экрана и файлы по USB идут сотнями МБ/с.
    t.stream_receive_window((16u32 << 20).into());
    t.receive_window((64u32 << 20).into());
    t.send_window(64 << 20);
    Arc::new(t)
}

pub fn endpoint(id: &Identity, port: u16) -> Result<quinn::Endpoint> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cert = CertificateDer::from(id.cert.clone());
    let key = || PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_der.clone()));
    let verifier = Arc::new(PinnedLater(provider.clone()));

    let mut server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier.clone())
        .with_single_cert(vec![cert.clone()], key())?;
    server.alpn_protocols = vec![ALPN.to_vec()];
    server.max_early_data_size = 0;
    let mut server_cfg = quinn::ServerConfig::with_crypto(Arc::new(quinn::crypto::rustls::QuicServerConfig::try_from(server)?));
    server_cfg.transport_config(transport());

    let mut client = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![cert], key())?;
    client.alpn_protocols = vec![ALPN.to_vec()];
    let mut client_cfg = quinn::ClientConfig::new(Arc::new(quinn::crypto::rustls::QuicClientConfig::try_from(client)?));
    client_cfg.transport_config(transport());

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let mut ep = quinn::Endpoint::server(server_cfg, addr).with_context(|| format!("UDP-порт {port}"))?;
    ep.set_default_client_config(client_cfg);
    Ok(ep)
}

/// Отпечаток сертификата собеседника.
pub fn peer_fingerprint(conn: &quinn::Connection) -> Option<String> {
    let any = conn.peer_identity()?;
    let certs = any.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().map(|c| crate::identity::fingerprint(c.as_ref()))
}
