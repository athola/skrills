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
#[cfg(feature = "http-transport")]
use std::path::Path;
use std::path::PathBuf;

/// Directory name for TLS certificates within ~/.skrills/
const TLS_DIR: &str = "tls";

/// Certificate filename
const CERT_FILENAME: &str = "cert.pem";

/// Private key filename
const KEY_FILENAME: &str = "key.pem";

/// Validity period for self-signed certificates (365 days)
const CERT_VALIDITY_DAYS: i64 = 365;

/// A certificate is regenerated once it is this close to expiring, so a
/// long-running server does not cross the expiry mid-session.
const CERT_RENEW_MARGIN_DAYS: i64 = 7;

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
/// # Expiry
/// Certificates are generated with a 365-day validity period. A pair whose
/// certificate file was written more than that period (less a 7-day margin)
/// ago is replaced with a fresh one. The key file's mode is reset to `0o600`
/// on every reuse.
///
/// # Errors
/// Returns an error if:
/// - Home directory cannot be determined
/// - TLS directory cannot be created
/// - Certificate generation fails
/// - File I/O fails
#[cfg(feature = "http-transport")]
pub fn ensure_auto_tls_certs() -> Result<(PathBuf, PathBuf)> {
    ensure_auto_tls_certs_in(&tls_dir()?)
}

#[cfg(feature = "http-transport")]
/// Whether the certificate at `cert_path` was written long enough ago that it
/// has expired or is about to. An unreadable timestamp counts as stale.
fn cert_is_stale(cert_path: &Path) -> bool {
    let max_age = std::time::Duration::from_secs(
        ((CERT_VALIDITY_DAYS - CERT_RENEW_MARGIN_DAYS) * 24 * 60 * 60) as u64,
    );
    fs::metadata(cert_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|written| written.elapsed().ok())
        .is_none_or(|age| age >= max_age)
}

#[cfg(feature = "http-transport")]
/// Restricts the private key to its owner.
fn restrict_key_permissions(key_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(key_path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!(
                "Failed to restrict permissions on TLS private key {}",
                key_path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = key_path;
    Ok(())
}

#[cfg(feature = "http-transport")]
fn ensure_auto_tls_certs_in(tls_path: &Path) -> Result<(PathBuf, PathBuf)> {
    let cert_path = tls_path.join(CERT_FILENAME);
    let key_path = tls_path.join(KEY_FILENAME);

    // Reuse an existing pair unless the certificate is near expiry.
    if cert_path.exists() && key_path.exists() {
        if !cert_is_stale(&cert_path) {
            restrict_key_permissions(&key_path)?;
            tracing::debug!(
                target: "skrills::tls",
                cert = %cert_path.display(),
                "Reusing existing auto-generated TLS certificate"
            );
            return Ok((cert_path, key_path));
        }
        tracing::info!(
            target: "skrills::tls",
            cert = %cert_path.display(),
            "Auto-generated TLS certificate has expired or is about to; regenerating"
        );
    }

    // Create directory if it doesn't exist
    fs::create_dir_all(tls_path)
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

    // `mode` above applies only when the file is created; an existing key
    // file keeps whatever mode it had.
    restrict_key_permissions(&key_path)?;

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
pub fn generate_self_signed_cert() -> Result<(String, String)> {
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
        SanType::DnsName("localhost".try_into().expect("static DNS literal")),
        SanType::DnsName("127.0.0.1".try_into().expect("static DNS literal")),
        SanType::DnsName("::1".try_into().expect("static DNS literal")),
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
        let path = result.expect("tls_dir should return a valid path");
        assert!(path.ends_with(".skrills/tls"));
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn generate_cert_produces_valid_pem() {
        let result = generate_self_signed_cert();
        let (cert, key) = result.expect("generate_self_signed_cert should succeed");

        // Verify PEM format
        assert!(cert.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(cert.ends_with("-----END CERTIFICATE-----\n"));
        assert!(key.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(key.ends_with("-----END PRIVATE KEY-----\n"));
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn ensure_auto_tls_certs_creates_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let tls_path = temp_dir.path().join("tls");

        let (cert_path, key_path) = ensure_auto_tls_certs_in(&tls_path).unwrap();

        assert_eq!(cert_path, tls_path.join(CERT_FILENAME));
        assert_eq!(key_path, tls_path.join(KEY_FILENAME));
        assert!(std::fs::read_to_string(&cert_path)
            .unwrap()
            .contains("BEGIN CERTIFICATE"));
        assert!(std::fs::read_to_string(&key_path)
            .unwrap()
            .contains("BEGIN PRIVATE KEY"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn ensure_auto_tls_certs_reuses_a_fresh_pair() {
        let temp_dir = tempfile::tempdir().unwrap();
        let tls_path = temp_dir.path().join("tls");
        let (cert_path, _) = ensure_auto_tls_certs_in(&tls_path).unwrap();
        let first = std::fs::read_to_string(&cert_path).unwrap();

        ensure_auto_tls_certs_in(&tls_path).unwrap();

        assert_eq!(std::fs::read_to_string(&cert_path).unwrap(), first);
    }

    /// Reuse used to check existence only, so an expired certificate was
    /// served indefinitely.
    #[test]
    #[cfg(feature = "http-transport")]
    fn ensure_auto_tls_certs_regenerates_an_expired_pair() {
        let temp_dir = tempfile::tempdir().unwrap();
        let tls_path = temp_dir.path().join("tls");
        let (cert_path, _) = ensure_auto_tls_certs_in(&tls_path).unwrap();
        let first = std::fs::read_to_string(&cert_path).unwrap();
        let written = std::time::SystemTime::now()
            - std::time::Duration::from_secs((CERT_VALIDITY_DAYS as u64 + 1) * 24 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&cert_path)
            .unwrap()
            .set_modified(written)
            .unwrap();

        ensure_auto_tls_certs_in(&tls_path).unwrap();

        assert_ne!(std::fs::read_to_string(&cert_path).unwrap(), first);
    }

    /// The `0o600` mode applied only when the key file was created.
    #[cfg(all(unix, feature = "http-transport"))]
    #[test]
    fn ensure_auto_tls_certs_tightens_a_reused_key() {
        use std::os::unix::fs::PermissionsExt;
        let temp_dir = tempfile::tempdir().unwrap();
        let tls_path = temp_dir.path().join("tls");
        let (_, key_path) = ensure_auto_tls_certs_in(&tls_path).unwrap();
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        ensure_auto_tls_certs_in(&tls_path).unwrap();

        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
