//! Auto-generated TLS certificate support for development.
//!
//! This module provides functionality to generate self-signed TLS certificates
//! for local development use. Certificates are stored in `~/.skrills/tls/` and
//! reused across server restarts.
//!
//! **Security Warning**: Self-signed certificates should only be used for local
//! development. For production use, obtain certificates from a trusted CA or
//! use Let's Encrypt/ACME.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Directory name for TLS certificates within ~/.skrills/
const TLS_DIR: &str = "tls";

/// Certificate filename
const CERT_FILENAME: &str = "cert.pem";

/// Private key filename
const KEY_FILENAME: &str = "key.pem";

/// Validity period for self-signed certificates (365 days)
const CERT_VALIDITY_DAYS: i64 = 365;

/// Returns the path to the TLS directory (~/.skrills/tls/).
fn tls_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".skrills").join(TLS_DIR))
}

/// Ensures auto-generated TLS certificates exist, creating them if necessary.
///
/// Returns the paths to the certificate and key files.
///
/// # Behavior
/// - If certificate files already exist, returns their paths (reuses existing)
/// - If certificates don't exist, generates new self-signed certificates
/// - Certificates are stored in `~/.skrills/tls/`
///
/// # Note
/// This function only checks for file existence, not certificate validity
/// or expiration. Certificates are generated with a 365-day validity period.
/// If certificates expire, delete the files to regenerate:
/// `rm ~/.skrills/tls/cert.pem ~/.skrills/tls/key.pem`
///
/// # Errors
/// Returns an error if:
/// - Home directory cannot be determined
/// - TLS directory cannot be created
/// - Certificate generation fails
/// - File I/O fails
#[cfg(feature = "http-transport")]
pub fn ensure_auto_tls_certs() -> Result<(PathBuf, PathBuf)> {
    let tls_path = tls_dir()?;
    let cert_path = tls_path.join(CERT_FILENAME);
    let key_path = tls_path.join(KEY_FILENAME);

    // Check if both files already exist
    if cert_path.exists() && key_path.exists() {
        tracing::debug!(
            target: "skrills::tls",
            cert = %cert_path.display(),
            "Reusing existing auto-generated TLS certificate"
        );
        return Ok((cert_path, key_path));
    }

    // Create directory if it doesn't exist
    fs::create_dir_all(&tls_path)
        .with_context(|| format!("Failed to create TLS directory at {}", tls_path.display()))?;

    // Generate new self-signed certificate
    tracing::info!(
        target: "skrills::tls",
        path = %tls_path.display(),
        "Generating self-signed TLS certificate for development"
    );

    let (cert_pem, key_pem) = generate_self_signed_cert()?;

    // Write certificate
    fs::write(&cert_path, &cert_pem)
        .with_context(|| format!("Failed to write TLS certificate to {}", cert_path.display()))?;

    // Write private key with restricted permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600) // Read/write for owner only
            .open(&key_path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, key_pem.as_bytes()))
            .with_context(|| {
                format!("Failed to write TLS private key to {}", key_path.display())
            })?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&key_path, &key_pem).with_context(|| {
            format!("Failed to write TLS private key to {}", key_path.display())
        })?;
    }

    tracing::info!(
        target: "skrills::tls",
        cert = %cert_path.display(),
        validity_days = CERT_VALIDITY_DAYS,
        "Self-signed TLS certificate generated successfully"
    );

    // Print user-friendly warning about self-signed certs
    eprintln!();
    eprintln!("╔═══════════════════════════════════════════════════════════════════╗");
    eprintln!("║  TLS: Using auto-generated self-signed certificate                ║");
    eprintln!("║                                                                   ║");
    eprintln!("║  ⚠️  Your browser will show a security warning. This is expected  ║");
    eprintln!("║     for self-signed certificates used in development.             ║");
    eprintln!("║                                                                   ║");
    eprintln!("║  For production, use proper certificates from a trusted CA.       ║");
    eprintln!("╚═══════════════════════════════════════════════════════════════════╝");
    eprintln!();

    Ok((cert_path, key_path))
}

/// Generates a self-signed certificate and private key.
///
/// Returns (certificate_pem, private_key_pem).
#[cfg(feature = "http-transport")]
fn generate_self_signed_cert() -> Result<(String, String)> {
    use rcgen::{CertificateParams, DnType, KeyPair, SanType};

    // Generate key pair
    let key_pair = KeyPair::generate().context("Failed to generate TLS key pair")?;

    // Configure certificate parameters
    let mut params = CertificateParams::default();

    // Set distinguished name
    params
        .distinguished_name
        .push(DnType::CommonName, "skrills localhost");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "skrills development");

    // Set Subject Alternative Names for localhost
    params.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into().unwrap()),
        SanType::DnsName("127.0.0.1".try_into().unwrap()),
        SanType::DnsName("::1".try_into().unwrap()),
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
        SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)),
    ];

    // Set validity period
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + time::Duration::days(CERT_VALIDITY_DAYS);

    // Generate certificate
    let cert = params
        .self_signed(&key_pair)
        .context("Failed to generate self-signed certificate")?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    Ok((cert_pem, key_pem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_dir_returns_expected_path() {
        let result = tls_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with(".skrills/tls"));
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn generate_cert_produces_valid_pem() {
        let result = generate_self_signed_cert();
        assert!(result.is_ok());
        let (cert, key) = result.unwrap();

        // Verify PEM format
        assert!(cert.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(cert.ends_with("-----END CERTIFICATE-----\n"));
        assert!(key.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(key.ends_with("-----END PRIVATE KEY-----\n"));
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn ensure_auto_tls_certs_creates_files() {
        // Use a temp directory to avoid polluting user's home
        let temp_dir = tempfile::tempdir().unwrap();
        let tls_path = temp_dir.path().join("tls");
        let cert_path = tls_path.join(CERT_FILENAME);
        let key_path = tls_path.join(KEY_FILENAME);

        // Create the directory
        std::fs::create_dir_all(&tls_path).unwrap();

        // Generate cert
        let (cert, key) = generate_self_signed_cert().unwrap();
        std::fs::write(&cert_path, &cert).unwrap();
        std::fs::write(&key_path, &key).unwrap();

        // Verify files exist
        assert!(cert_path.exists());
        assert!(key_path.exists());

        // Verify content
        let read_cert = std::fs::read_to_string(&cert_path).unwrap();
        let read_key = std::fs::read_to_string(&key_path).unwrap();
        assert!(read_cert.contains("BEGIN CERTIFICATE"));
        assert!(read_key.contains("BEGIN PRIVATE KEY"));
    }
}
