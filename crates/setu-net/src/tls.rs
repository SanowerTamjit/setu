//! TLS certificate check: who a server's certificate is for, who signed
//! it, when it expires, and whether this computer would trust it.
//!
//! The handshake is made with rustls over any byte stream: a TCP connection
//! from here ([`check`]) or a channel through an SSH server ([`check_over`]).
//! The certificate is checked against the system's trusted roots, but a
//! failed check does not stop the handshake: it is recorded and reported,
//! because "the certificate expired" is the answer people are looking for.
//! Nothing is sent after the handshake; the connection is closed at once.
//!
//! Certificates come from the server and are **untrusted**: they are parsed
//! with `x509-cert`, every text taken from them is cleaned of control
//! characters and shortened, and at most [`MAX_CHAIN`] are read.

use crate::{check_target, NetError};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use x509_cert::der::Decode;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::Certificate;

/// Most certificates read from a chain.
pub const MAX_CHAIN: usize = 8;
/// Most names listed for one certificate.
const MAX_NAMES: usize = 50;
const MAX_TEXT: usize = 300;

/// One certificate of the chain the server sent, leaf first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    /// DNS names and addresses it is valid for (subjectAltName).
    pub names: Vec<String>,
    /// Seconds since 1970.
    pub not_before: u64,
    pub not_after: u64,
    /// Whole days until it expires (negative once it has).
    pub days_left: i64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsReport {
    /// Whether this computer would trust the certificate for this name.
    pub trusted: bool,
    /// Why not, in words.
    pub problem: Option<String>,
    /// "TLS 1.3" or "TLS 1.2".
    pub protocol: String,
    pub cipher: String,
    pub chain: Vec<CertInfo>,
}

/// What verification found, filled in during the handshake.
#[derive(Debug)]
struct Recording {
    inner: Arc<WebPkiServerVerifier>,
    result: Mutex<Option<Result<(), rustls::Error>>>,
}

impl ServerCertVerifier for Recording {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let result = self
            .inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .map(|_| ());
        if let Ok(mut slot) = self.result.lock() {
            *slot = Some(result);
        }
        // Go on either way: the report says whether it would be trusted.
        Ok(ServerCertVerified::assertion())
    }

    // Signatures are still checked, so the chain shown is the one the
    // server holds the key for.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// The system's trusted roots, loaded once.
pub fn system_roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    Arc::clone(ROOTS.get_or_init(|| {
        let mut store = RootCertStore::empty();
        let found = rustls_native_certs::load_native_certs();
        let _ = store.add_parsable_certificates(found.certs);
        Arc::new(store)
    }))
}

/// Connects to `host:port` from this computer and looks at its certificate.
/// `name` is the name to check it for (SNI); `host` when empty.
pub async fn check(
    host: &str,
    port: u16,
    name: &str,
    timeout: Duration,
) -> Result<TlsReport, NetError> {
    let host = check_target(host)?;
    let tcp = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| NetError::Tls(format!("no answer from {host}:{port} in time")))?
    .map_err(|e| NetError::Tls(format!("could not connect to {host}:{port}: {e}")))?;
    let name = if name.trim().is_empty() {
        host.as_str()
    } else {
        name
    };
    check_over(tcp, name, timeout, system_roots()).await
}

/// Makes a TLS handshake over `stream` for `name` and reports the
/// certificate, checked against `roots`.
pub async fn check_over<S>(
    stream: S,
    name: &str,
    timeout: Duration,
    roots: Arc<RootCertStore>,
) -> Result<TlsReport, NetError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let name = check_target(name)?;
    let server_name =
        ServerName::try_from(name.clone()).map_err(|_| NetError::BadTarget(name.clone()))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner = WebPkiServerVerifier::builder_with_provider(roots, Arc::clone(&provider))
        .build()
        .map_err(|e| NetError::Tls(format!("no trusted certificates on this computer ({e})")))?;
    let recording = Arc::new(Recording {
        inner,
        result: Mutex::new(None),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| NetError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&recording) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let mut tls = tokio::time::timeout(timeout, connector.connect(server_name, stream))
        .await
        .map_err(|_| NetError::Tls("the TLS handshake did not finish in time".into()))?
        .map_err(|e| NetError::Tls(handshake_problem(&e)))?;
    let (_, conn) = tls.get_ref();
    let protocol = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3".to_string(),
        Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2".to_string(),
        Some(v) => format!("{v:?}"),
        None => "unknown".into(),
    };
    let cipher = conn
        .negotiated_cipher_suite()
        .map(|c| format!("{:?}", c.suite()))
        .unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let chain: Vec<CertInfo> = conn
        .peer_certificates()
        .unwrap_or_default()
        .iter()
        .take(MAX_CHAIN)
        .filter_map(|c| describe(c.as_ref(), now))
        .collect();
    let _ = tokio::time::timeout(Duration::from_secs(2), tls.shutdown()).await;
    let verdict = recording.result.lock().ok().and_then(|mut r| r.take());
    let (trusted, problem) = match verdict {
        Some(Ok(())) => (true, None),
        Some(Err(e)) => (false, Some(trust_problem(&e, &name))),
        None => (false, Some("the server sent no certificate".into())),
    };
    if chain.is_empty() {
        return Err(NetError::Tls(
            "the server sent no certificate Setu could read".into(),
        ));
    }
    // A lone certificate that signed itself: say so plainly, whatever
    // detail the verifier tripped on first.
    let problem = match problem {
        Some(_) if chain.len() == 1 && chain[0].subject == chain[0].issuer => Some(
            "the certificate is self-signed, so no authority this computer trusts vouches for it"
                .to_string(),
        ),
        other => other,
    };
    Ok(TlsReport {
        trusted,
        problem,
        protocol,
        cipher,
        chain,
    })
}

/// Printable characters only, shortened.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect()
}

/// Reads one DER certificate; `None` if it cannot be parsed.
pub fn describe(der: &[u8], now_secs: u64) -> Option<CertInfo> {
    let cert = Certificate::from_der(der).ok()?;
    let tbs = cert.tbs_certificate();
    let validity = tbs.validity();
    let not_before = validity.not_before.to_unix_duration().as_secs();
    let not_after = validity.not_after.to_unix_duration().as_secs();
    let days_left = (i128::from(not_after) - i128::from(now_secs)).div_euclid(86_400);
    let mut names = Vec::new();
    if let Ok(Some((_, SubjectAltName(general)))) = tbs.get_extension::<SubjectAltName>() {
        for n in general.iter().take(MAX_NAMES) {
            match n {
                GeneralName::DnsName(d) => names.push(clean(d.as_str())),
                GeneralName::IpAddress(ip) => {
                    let b = ip.as_bytes();
                    if let Ok(v4) = <[u8; 4]>::try_from(b) {
                        names.push(std::net::Ipv4Addr::from(v4).to_string());
                    } else if let Ok(v6) = <[u8; 16]>::try_from(b) {
                        names.push(std::net::Ipv6Addr::from(v6).to_string());
                    }
                }
                _ => {}
            }
        }
    }
    Some(CertInfo {
        subject: clean(&tbs.subject().to_string()),
        issuer: clean(&tbs.issuer().to_string()),
        names,
        not_before,
        not_after,
        days_left: i64::try_from(days_left).unwrap_or(i64::MIN),
        sha256: Sha256::digest(der)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    })
}

fn handshake_problem(e: &std::io::Error) -> String {
    let text = e.to_string();
    if text.contains("InvalidContentType") || text.contains("corrupt message") {
        "the server did not answer with TLS (is this the right port?)".into()
    } else if e.kind() == std::io::ErrorKind::UnexpectedEof {
        "the server closed the connection during the TLS handshake".into()
    } else {
        format!("the TLS handshake failed: {}", clean(&text))
    }
}

/// Why a certificate would not be trusted, in words.
fn trust_problem(e: &rustls::Error, name: &str) -> String {
    match e {
        rustls::Error::InvalidCertificate(c) => match c {
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                "the certificate has expired".into()
            }
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                "the certificate is not valid yet (or this computer's clock is wrong)".into()
            }
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                format!("the certificate is not for {name}")
            }
            CertificateError::UnknownIssuer => {
                "the certificate is not signed by an authority this computer trusts (self-signed, a private CA, or a missing intermediate certificate)".into()
            }
            CertificateError::Revoked => "the certificate has been revoked".into(),
            CertificateError::BadSignature => "the certificate's signature is not valid".into(),
            other => format!("the certificate is not valid ({other:?})"),
        },
        other => format!("the certificate could not be checked ({other})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_are_in_words() {
        let e = rustls::Error::InvalidCertificate(CertificateError::Expired);
        assert_eq!(trust_problem(&e, "x"), "the certificate has expired");
        let e = rustls::Error::InvalidCertificate(CertificateError::NotValidForName);
        assert_eq!(
            trust_problem(&e, "db.internal"),
            "the certificate is not for db.internal"
        );
        let e = rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer);
        assert!(trust_problem(&e, "x").contains("self-signed"));
    }

    /// A self-signed test certificate (public part only), valid
    /// 2026-11-25 to 2126-11-01, for fixture.example.
    const FIXTURE: &[u8] = include_bytes!("testdata/fixture.der");

    #[test]
    fn certificates_are_described() {
        let at = 1_795_000_000; // 2026-11-18, before it starts
        let c = describe(FIXTURE, at).expect("parses");
        assert!(c.subject.contains("CN=fixture.example"), "{}", c.subject);
        assert_eq!(c.subject, c.issuer, "self-signed");
        assert_eq!(
            c.names,
            vec!["fixture.example", "www.fixture.example", "10.0.0.5"]
        );
        assert!(c.not_before < c.not_after);
        assert!(c.days_left > 36_000);
        assert!(c.sha256.starts_with("FF:25:78:C0"), "{}", c.sha256);
        // After it expires, days left goes negative.
        assert!(
            describe(FIXTURE, c.not_after + 3 * 86_400)
                .expect("parses")
                .days_left
                <= -3
        );
    }

    #[test]
    fn garbage_is_not_a_certificate() {
        assert_eq!(describe(b"not a certificate", 0), None);
        assert_eq!(describe(&[], 0), None);
    }
}
