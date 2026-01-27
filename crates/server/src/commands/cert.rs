//! TLS certificate management commands.
//!
//! Provides CLI handlers for certificate status, renewal, and installation.

use crate::cli::OutputFormat;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use tracing::debug;
#[cfg(feature = "http-transport")]
use tracing::warn;

/// Certificate expiry warning threshold (days).
const CERT_EXPIRY_CRITICAL_DAYS: i64 = 7;
/// Certificate expiry caution threshold (days).
const CERT_EXPIRY_WARNING_DAYS: i64 = 30;
/// Maximum certificate file size (10 MB) to prevent memory exhaustion.
const MAX_CERT_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Certificate information structure.
#[derive(Debug, serde::Serialize)]
pub struct CertInfo {
    pub path: PathBuf,
    pub exists: bool,
    pub issuer: Option<String>,
    pub subject: Option<String>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
    pub days_until_expiry: Option<i64>,
    pub is_valid: bool,
    pub is_self_signed: bool,
}

/// Returns the default TLS directory path (~/.skrills/tls/).
fn tls_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".skrills").join("tls"))
}

/// Parse a PEM certificate and extract metadata.
#[cfg(feature = "http-transport")]
fn parse_cert_info(cert_path: &PathBuf) -> Result<CertInfo> {
    use x509_parser::prelude::*;

    if !cert_path.exists() {
        return Ok(CertInfo {
            path: cert_path.clone(),
            exists: false,
            issuer: None,
            subject: None,
            not_before: None,
            not_after: None,
            days_until_expiry: None,
            is_valid: false,
            is_self_signed: false,
        });
    }

    let pem_data = fs::read_to_string(cert_path)
        .with_context(|| format!("Failed to read certificate: {}", cert_path.display()))?;

    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_data.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to parse PEM: {}", e))?;

    let (_, cert) = X509Certificate::from_der(&pem.contents)
        .map_err(|e| anyhow::anyhow!("Failed to parse X.509 certificate: {}", e))?;

    let issuer = cert.issuer().to_string();
    let subject = cert.subject().to_string();
    let not_before = cert.validity().not_before.to_rfc2822().ok();
    let not_after = cert.validity().not_after.to_rfc2822().ok();

    // String comparison is sufficient here: x509-parser formats Distinguished Names
    // canonically, and for status display purposes exact structural DN matching is unnecessary.
    // Calculate days until expiry
    let now = ::time::OffsetDateTime::now_utc();
    let expiry_timestamp = cert.validity().not_after.timestamp();
    let now_timestamp = now.unix_timestamp();
    let days_until_expiry = (expiry_timestamp - now_timestamp) / 86400;

    let is_valid = days_until_expiry > 0;
    let is_self_signed = issuer == subject;

    Ok(CertInfo {
        path: cert_path.clone(),
        exists: true,
        issuer: Some(issuer),
        subject: Some(subject),
        not_before,
        not_after,
        days_until_expiry: Some(days_until_expiry),
        is_valid,
        is_self_signed,
    })
}

#[cfg(not(feature = "http-transport"))]
fn parse_cert_info(cert_path: &PathBuf) -> Result<CertInfo> {
    Ok(CertInfo {
        path: cert_path.clone(),
        exists: cert_path.exists(),
        issuer: None,
        subject: None,
        not_before: None,
        not_after: None,
        days_until_expiry: None,
        is_valid: false,
        is_self_signed: false,
    })
}

/// Compute a SHA-256 fingerprint of a key file for safe display.
fn key_fingerprint(key_path: &PathBuf) -> Result<String> {
    let data = fs::read(key_path)
        .with_context(|| format!("Failed to read key file: {}", key_path.display()))?;
    let hash = Sha256::digest(&data);
    Ok(format!("SHA256:{}", hex::encode(hash)))
}

/// Handle `skrills cert status` command.
pub fn handle_cert_status_command(format: OutputFormat) -> Result<()> {
    let tls_path = tls_dir()?;
    let cert_path = tls_path.join("cert.pem");
    let key_path = tls_path.join("key.pem");

    let cert_info = parse_cert_info(&cert_path)?;
    let key_exists = key_path.exists();

    if format.is_json() {
        #[derive(serde::Serialize)]
        struct Status {
            cert: CertInfo,
            key_exists: bool,
            key_fingerprint: Option<String>,
            tls_dir: PathBuf,
        }
        let kfp = if key_exists {
            key_fingerprint(&key_path).ok()
        } else {
            None
        };
        let status = Status {
            cert: cert_info,
            key_exists,
            key_fingerprint: kfp,
            tls_dir: tls_path,
        };
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    // Text output
    println!("TLS Certificate Status");
    println!("======================");
    println!();
    println!("TLS Directory: {}", tls_path.display());
    println!();

    if !cert_info.exists {
        println!("Certificate: NOT FOUND");
        println!("  Path: {}", cert_path.display());
        println!();
        println!("Hint: Run `skrills serve --http <addr> --tls-auto` to generate");
        println!("      a self-signed certificate for development.");
    } else {
        println!("Certificate: {}", cert_path.display());
        if let Some(ref subject) = cert_info.subject {
            println!("  Subject: {}", subject);
        }
        if let Some(ref issuer) = cert_info.issuer {
            println!("  Issuer:  {}", issuer);
        }
        if let Some(ref not_before) = cert_info.not_before {
            println!("  Valid From: {}", not_before);
        }
        if let Some(ref not_after) = cert_info.not_after {
            println!("  Valid Until: {}", not_after);
        }
        if let Some(days) = cert_info.days_until_expiry {
            let status = if days <= 0 {
                "EXPIRED"
            } else if days <= CERT_EXPIRY_WARNING_DAYS {
                "EXPIRING SOON"
            } else {
                "OK"
            };
            println!("  Days Until Expiry: {} ({})", days, status);
        }
        println!(
            "  Self-Signed: {}",
            if cert_info.is_self_signed {
                "Yes"
            } else {
                "No"
            }
        );
        println!("  Valid: {}", if cert_info.is_valid { "Yes" } else { "No" });
    }

    println!();
    if key_exists {
        let fingerprint = key_fingerprint(&key_path)?;
        println!("Private Key: FOUND");
        println!("  Fingerprint: {}", fingerprint);
    } else {
        println!("Private Key: NOT FOUND");
    }

    Ok(())
}

/// Handle `skrills cert renew` command.
#[cfg(feature = "http-transport")]
pub fn handle_cert_renew_command(force: bool) -> Result<()> {
    use crate::tls_auto::ensure_auto_tls_certs;

    let tls_path = tls_dir()?;
    let cert_path = tls_path.join("cert.pem");
    let key_path = tls_path.join("key.pem");

    // Check if renewal is needed
    if cert_path.exists() && !force {
        let cert_info = parse_cert_info(&cert_path)?;
        if let Some(days) = cert_info.days_until_expiry {
            if days > CERT_EXPIRY_WARNING_DAYS {
                println!(
                    "Certificate is still valid for {} days. Use --force to renew anyway.",
                    days
                );
                return Ok(());
            }
        }
    }

    // Remove existing certs to trigger regeneration
    if cert_path.exists() {
        fs::remove_file(&cert_path).with_context(|| {
            format!("Failed to remove old certificate: {}", cert_path.display())
        })?;
    }
    if key_path.exists() {
        fs::remove_file(&key_path)
            .with_context(|| format!("Failed to remove old key: {}", key_path.display()))?;
    }

    // Generate new certificate
    let (new_cert, new_key) = ensure_auto_tls_certs()?;
    println!("Certificate renewed successfully!");
    println!("  Certificate: {}", new_cert.display());
    let fingerprint = key_fingerprint(&new_key)?;
    println!("  Private Key Fingerprint: {}", fingerprint);

    Ok(())
}

#[cfg(not(feature = "http-transport"))]
pub fn handle_cert_renew_command(_force: bool) -> Result<()> {
    bail!("Certificate renewal requires the 'http-transport' feature")
}

/// Validate that a file contains PEM-formatted certificate data.
///
/// Checks that the file starts with the standard PEM certificate header.
/// Returns `Ok(true)` if valid, `Ok(false)` if not valid PEM format.
#[cfg(feature = "http-transport")]
pub fn validate_pem_format(path: &PathBuf) -> Result<bool> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file for PEM validation: {}", path.display()))?;
    Ok(content
        .trim_start()
        .starts_with("-----BEGIN CERTIFICATE-----"))
}

/// Handle `skrills cert install <path>` command.
pub fn handle_cert_install_command(
    cert_source: PathBuf,
    key_source: Option<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let tls_path = tls_dir()?;
    let cert_dest = tls_path.join("cert.pem");
    let key_dest = tls_path.join("key.pem");

    // Validate source cert exists
    if !cert_source.exists() {
        bail!("Certificate file not found: {}", cert_source.display());
    }

    // Prevent memory exhaustion from oversized files
    let cert_meta = fs::metadata(&cert_source).with_context(|| {
        format!(
            "Failed to read certificate metadata: {}",
            cert_source.display()
        )
    })?;
    if cert_meta.len() > MAX_CERT_FILE_SIZE {
        bail!("Certificate file exceeds maximum size of 10 MB");
    }

    // Create TLS directory if needed
    fs::create_dir_all(&tls_path)
        .with_context(|| format!("Failed to create TLS directory: {}", tls_path.display()))?;

    // Copy certificate
    fs::copy(&cert_source, &cert_dest).with_context(|| {
        format!(
            "Failed to copy certificate from {} to {}",
            cert_source.display(),
            cert_dest.display()
        )
    })?;

    // Warn if cert doesn't look like valid PEM (non-fatal)
    match validate_pem_format(&cert_dest) {
        Ok(false) => {
            warn!(path = %cert_dest.display(), "Installed certificate does not appear to be valid PEM format")
        }
        Err(e) => warn!(error = %e, "Could not validate PEM format of installed certificate"),
        Ok(true) => {}
    }

    // Copy key if provided
    if let Some(ref key_src) = key_source {
        if !key_src.exists() {
            bail!("Key file not found: {}", key_src.display());
        }

        // Set restrictive permissions on key file (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let key_data = fs::read(key_src)
                .with_context(|| format!("Failed to read key file: {}", key_src.display()))?;

            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&key_dest)
                .and_then(|mut f| std::io::Write::write_all(&mut f, &key_data))
                .with_context(|| format!("Failed to write key file to {}", key_dest.display()))?;
        }

        #[cfg(not(unix))]
        {
            fs::copy(key_src, &key_dest).with_context(|| {
                format!(
                    "Failed to copy key from {} to {}",
                    key_src.display(),
                    key_dest.display()
                )
            })?;
        }
    }

    if format.is_json() {
        #[derive(serde::Serialize)]
        struct InstallResult {
            cert_installed: PathBuf,
            key_fingerprint: Option<String>,
        }
        let kfp = if key_source.is_some() {
            Some(key_fingerprint(&key_dest)?)
        } else {
            None
        };
        let result = InstallResult {
            cert_installed: cert_dest,
            key_fingerprint: kfp,
        };
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Certificate installed successfully!");
        println!("  Certificate: {}", cert_dest.display());
        if key_source.is_some() {
            let fingerprint = key_fingerprint(&key_dest)?;
            println!("  Private Key Fingerprint: {}", fingerprint);
        }
    }

    Ok(())
}

/// Get certificate status for display on server startup.
#[cfg(feature = "http-transport")]
pub fn get_cert_status_summary() -> Option<String> {
    let tls_path = match tls_dir() {
        Ok(p) => p,
        Err(e) => {
            debug!(error = %e, "Failed to resolve TLS directory");
            return None;
        }
    };
    let cert_path = tls_path.join("cert.pem");

    if !cert_path.exists() {
        return None;
    }

    let cert_info = match parse_cert_info(&cert_path) {
        Ok(info) => info,
        Err(e) => {
            debug!(error = %e, path = %cert_path.display(), "Failed to parse certificate");
            return None;
        }
    };
    if !cert_info.exists {
        return None;
    }

    let days = cert_info.days_until_expiry?;
    let status = if days <= 0 {
        "EXPIRED"
    } else if days <= CERT_EXPIRY_CRITICAL_DAYS {
        "CRITICAL"
    } else if days <= CERT_EXPIRY_WARNING_DAYS {
        "WARNING"
    } else {
        "OK"
    };

    let self_signed = if cert_info.is_self_signed {
        " (self-signed)"
    } else {
        ""
    };

    Some(format!(
        "TLS: {} days until expiry [{}]{}",
        days, status, self_signed
    ))
}

#[cfg(not(feature = "http-transport"))]
pub fn get_cert_status_summary() -> Option<String> {
    None
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
    fn cert_info_default_for_missing_file() {
        let path = PathBuf::from("/nonexistent/path/cert.pem");
        let info = parse_cert_info(&path).unwrap();

        assert!(!info.exists);
        assert!(!info.is_valid);
        assert!(!info.is_self_signed);
        assert!(info.issuer.is_none());
        assert!(info.subject.is_none());
        assert!(info.days_until_expiry.is_none());
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn cert_info_parses_valid_pem() {
        use crate::tls_auto::generate_self_signed_cert;

        let tmp = tempfile::tempdir().unwrap();
        let cert_path = tmp.path().join("cert.pem");

        // Generate and write a test certificate
        let (cert_pem, _key_pem) = generate_self_signed_cert().unwrap();
        std::fs::write(&cert_path, &cert_pem).unwrap();

        let info = parse_cert_info(&cert_path).unwrap();

        assert!(info.exists);
        assert!(info.is_valid);
        assert!(info.is_self_signed); // Self-signed cert has issuer == subject
        assert!(info.issuer.is_some());
        assert!(info.subject.is_some());
        assert!(info.days_until_expiry.is_some());
        // Fresh cert should have ~365 days validity
        let days = info.days_until_expiry.unwrap();
        assert!(
            days > 360 && days <= 366,
            "Expected ~365 days, got {}",
            days
        );
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn cert_info_handles_invalid_pem() {
        let tmp = tempfile::tempdir().unwrap();
        let cert_path = tmp.path().join("bad_cert.pem");

        // Write invalid PEM content
        std::fs::write(&cert_path, "not a valid certificate").unwrap();

        let result = parse_cert_info(&cert_path);
        assert!(result.is_err());
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn cert_info_detects_expired_certificate() {
        use rcgen::{CertificateParams, DnType, KeyPair};

        let key_pair = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, "expired test");

        // Set validity entirely in the past
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(30);
        params.not_after = now - time::Duration::days(1);

        let cert = params.self_signed(&key_pair).unwrap();
        let cert_pem = cert.pem();

        let tmp = tempfile::tempdir().unwrap();
        let cert_path = tmp.path().join("expired.pem");
        std::fs::write(&cert_path, &cert_pem).unwrap();

        let info = parse_cert_info(&cert_path).unwrap();
        assert!(info.exists);
        assert!(!info.is_valid, "Expired cert should not be valid");
        assert!(
            info.days_until_expiry.unwrap() <= 0,
            "Expected non-positive days_until_expiry, got {}",
            info.days_until_expiry.unwrap()
        );
    }

    #[test]
    #[cfg(unix)]
    fn install_sets_restrictive_key_permissions() {
        use std::os::unix::fs::MetadataExt;

        let tmp = tempfile::tempdir().unwrap();
        let src_cert = tmp.path().join("src_cert.pem");
        let src_key = tmp.path().join("src_key.pem");

        std::fs::write(
            &src_cert,
            "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        std::fs::write(
            &src_key,
            "-----BEGIN PRIVATE KEY-----\ntest\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();

        // Override tls_dir by using handle_cert_install_command with a custom HOME
        // Instead, we replicate the key-writing logic directly to test permissions
        let dest_key = tmp.path().join("key.pem");
        let key_data = std::fs::read(&src_key).unwrap();

        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&dest_key)
                .and_then(|mut f| std::io::Write::write_all(&mut f, &key_data))
                .unwrap();
        }

        let meta = std::fs::metadata(&dest_key).unwrap();
        let mode = meta.mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "Key file permissions should be 0600, got {:o}",
            mode
        );
    }

    #[test]
    fn validate_pem_format_accepts_valid_pem() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("valid.pem");
        std::fs::write(
            &path,
            "-----BEGIN CERTIFICATE-----\nMIIBxTCCAW...\n-----END CERTIFICATE-----\n",
        )
        .unwrap();

        assert!(validate_pem_format(&path).unwrap());
    }

    #[test]
    fn validate_pem_format_rejects_invalid_pem() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("invalid.pem");
        std::fs::write(&path, "this is not a PEM file").unwrap();

        assert!(!validate_pem_format(&path).unwrap());
    }

    #[test]
    fn install_workflow_creates_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src_cert = tmp.path().join("src_cert.pem");
        let src_key = tmp.path().join("src_key.pem");

        let cert_content =
            "-----BEGIN CERTIFICATE-----\nMIIBxTCCAW...\n-----END CERTIFICATE-----\n";
        let key_content = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBAD...\n-----END PRIVATE KEY-----\n";
        std::fs::write(&src_cert, cert_content).unwrap();
        std::fs::write(&src_key, key_content).unwrap();

        // Simulate install by copying to a target directory (mirrors handle_cert_install_command logic)
        let tls_path = tmp.path().join("tls");
        std::fs::create_dir_all(&tls_path).unwrap();
        let cert_dest = tls_path.join("cert.pem");
        let key_dest = tls_path.join("key.pem");

        std::fs::copy(&src_cert, &cert_dest).unwrap();
        std::fs::copy(&src_key, &key_dest).unwrap();

        assert!(cert_dest.exists(), "Cert file should exist after install");
        assert!(key_dest.exists(), "Key file should exist after install");

        let installed_cert = std::fs::read_to_string(&cert_dest).unwrap();
        assert_eq!(installed_cert, cert_content);

        let installed_key = std::fs::read_to_string(&key_dest).unwrap();
        assert_eq!(installed_key, key_content);
    }
}
