//! TLS certificate management commands.
//!
//! Provides CLI handlers for certificate status, renewal, and installation.

use crate::cli::OutputFormat;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(feature = "http-transport")]
use tracing::debug;

/// Certificate expiry warning threshold (days).
#[cfg(feature = "http-transport")]
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
    let home = skrills_state::home_dir()?;
    Ok(home.join(".skrills").join("tls"))
}

/// Validity of a certificate whose window is `[not_before, not_after]`, at
/// `now` (all Unix seconds): whether it is currently valid, and the whole days
/// left, rounded down, so 23 hours left is 0 days but still valid.
#[cfg_attr(not(feature = "http-transport"), allow(dead_code))]
fn validity_at(not_before: i64, not_after: i64, now: i64) -> (bool, i64) {
    let is_valid = now >= not_before && now < not_after;
    (is_valid, (not_after - now).div_euclid(86_400))
}

/// Parse a PEM certificate and extract metadata.
#[cfg(feature = "http-transport")]
fn parse_cert_info(cert_path: &Path) -> Result<CertInfo> {
    use x509_parser::prelude::*;

    if !cert_path.exists() {
        return Ok(CertInfo {
            path: cert_path.to_path_buf(),
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

    let pem_data = read_limited(cert_path)?;

    let (_, pem) = x509_parser::pem::parse_x509_pem(&pem_data)
        .map_err(|e| anyhow::anyhow!("Failed to parse PEM: {}", e))?;

    let (_, cert) = X509Certificate::from_der(&pem.contents)
        .map_err(|e| anyhow::anyhow!("Failed to parse X.509 certificate: {}", e))?;

    let issuer = cert.issuer().to_string();
    let subject = cert.subject().to_string();
    let not_before = cert.validity().not_before.to_rfc2822().ok();
    let not_after = cert.validity().not_after.to_rfc2822().ok();

    // Compared as timestamps: a whole-day count said a certificate with 23
    // hours left had expired, and `not_before` was never checked.
    let (is_valid, days_until_expiry) = validity_at(
        cert.validity().not_before.timestamp(),
        cert.validity().not_after.timestamp(),
        ::time::OffsetDateTime::now_utc().unix_timestamp(),
    );
    // String comparison is sufficient here: x509-parser formats Distinguished
    // Names canonically.
    let is_self_signed = issuer == subject;

    Ok(CertInfo {
        path: cert_path.to_path_buf(),
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
fn parse_cert_info(cert_path: &Path) -> Result<CertInfo> {
    Ok(CertInfo {
        path: cert_path.to_path_buf(),
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

/// Reads `path`, refusing anything over [`MAX_CERT_FILE_SIZE`].
///
/// The limit is applied to the bytes read, not the reported length: a
/// special file such as `/dev/zero` or a FIFO reports length 0.
fn read_limited(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file =
        fs::File::open(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let mut data = Vec::new();
    file.take(MAX_CERT_FILE_SIZE + 1)
        .read_to_end(&mut data)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    if data.len() as u64 > MAX_CERT_FILE_SIZE {
        bail!("{} exceeds the maximum size of 10 MB", path.display());
    }
    Ok(data)
}

/// Compute a SHA-256 fingerprint of a key file for safe display.
fn key_fingerprint(key_path: &Path) -> Result<String> {
    let data = fs::read(key_path)
        .with_context(|| format!("Failed to read key file: {}", key_path.display()))?;
    let hash = Sha256::digest(&data);
    Ok(format!("SHA256:{}", hex::encode(hash)))
}

/// Creates the TLS directory, owner-only on Unix.
fn create_tls_dir(tls_path: &Path) -> Result<()> {
    fs::create_dir_all(tls_path)
        .with_context(|| format!("Failed to create TLS directory: {}", tls_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tls_path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("Failed to restrict TLS directory: {}", tls_path.display()))?;
    }
    Ok(())
}

/// Writes `data` to a fresh temporary file beside `dest`, owner-only when
/// `private`, and returns its path. The file is created new, so its mode is
/// the one asked for even when `dest` already exists with a looser one.
fn write_temp(dest: &Path, data: &[u8], private: bool) -> Result<PathBuf> {
    let tmp = dest.with_extension("pem.tmp");
    let _ = fs::remove_file(&tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if private { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = private;
    options
        .open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, data))
        .with_context(|| format!("Failed to write {}", tmp.display()))?;
    Ok(tmp)
}

/// Installs a certificate and, when given, its key into `tls_path`.
///
/// Both are written to temporary files first and renamed into place only
/// once both writes succeed, so a failure leaves the installed pair as it
/// was. The renamed key keeps the 0600 mode of the new file, replacing a
/// key.pem that was previously world-readable. If the certificate rename
/// fails after the key was replaced, the old key is put back, so the pair
/// on disk still matches.
fn install_pair(tls_path: &Path, cert_pem: &[u8], key_pem: Option<&[u8]>) -> Result<()> {
    create_tls_dir(tls_path)?;
    let cert_dest = tls_path.join("cert.pem");
    let key_dest = tls_path.join("key.pem");

    let cert_tmp = write_temp(&cert_dest, cert_pem, false)?;
    let key_tmp = match key_pem.map(|k| write_temp(&key_dest, k, true)).transpose() {
        Ok(k) => k,
        Err(e) => {
            remove_temp(&cert_tmp);
            return Err(e);
        }
    };

    let mut old_key = None;
    if let Some(key_tmp) = key_tmp {
        old_key = fs::read(&key_dest).ok();
        if let Err(e) = fs::rename(&key_tmp, &key_dest) {
            remove_temp(&key_tmp);
            remove_temp(&cert_tmp);
            return Err(e)
                .with_context(|| format!("Failed to install key at {}", key_dest.display()));
        }
    }
    if let Err(e) = fs::rename(&cert_tmp, &cert_dest) {
        remove_temp(&cert_tmp);
        if let Some(old) = old_key {
            let restored = write_temp(&key_dest, &old, true)
                .and_then(|tmp| fs::rename(&tmp, &key_dest).map_err(anyhow::Error::from));
            if let Err(restore_err) = restored {
                return Err(e).with_context(|| {
                    format!(
                        "Failed to install certificate at {}, and restoring the previous key failed too ({restore_err:#}); {} no longer matches",
                        cert_dest.display(),
                        key_dest.display()
                    )
                });
            }
        }
        return Err(e)
            .with_context(|| format!("Failed to install certificate at {}", cert_dest.display()));
    }
    Ok(())
}

/// Removes a temp file this module wrote; a failure to remove it is
/// reported but does not replace the error being returned.
fn remove_temp(tmp: &Path) {
    if let Err(e) = fs::remove_file(tmp) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("Warning: could not remove {}: {e}", tmp.display());
        }
    }
}

/// Whether `key_path` exists, its fingerprint, and why it could not be
/// read, so both status formats report an unreadable key the same way.
fn key_status(key_path: &Path) -> (bool, Option<String>, Option<String>) {
    if !key_path.exists() {
        return (false, None, None);
    }
    match key_fingerprint(key_path) {
        Ok(fp) => (true, Some(fp), None),
        Err(e) => (true, None, Some(format!("{e:#}"))),
    }
}

/// Handle `skrills cert status` command.
pub fn handle_cert_status_command(format: OutputFormat) -> Result<()> {
    let tls_path = tls_dir()?;
    let cert_path = tls_path.join("cert.pem");
    let key_path = tls_path.join("key.pem");

    let cert_info = parse_cert_info(&cert_path)?;
    let (key_exists, key_fp, key_error) = key_status(&key_path);

    if format.is_json() {
        #[derive(serde::Serialize)]
        struct Status {
            cert: CertInfo,
            key_exists: bool,
            key_fingerprint: Option<String>,
            key_error: Option<String>,
            tls_dir: PathBuf,
        }
        let status = Status {
            cert: cert_info,
            key_exists,
            key_fingerprint: key_fp,
            key_error,
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
            let status = if !cert_info.is_valid {
                "NOT VALID"
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
        println!("Private Key: FOUND");
        match (key_fp, key_error) {
            (Some(fingerprint), _) => println!("  Fingerprint: {}", fingerprint),
            (None, Some(error)) => println!("  Unreadable: {}", error),
            (None, None) => {}
        }
    } else {
        println!("Private Key: NOT FOUND");
    }

    Ok(())
}

/// Handle `skrills cert renew` command.
///
/// Renewal replaces the pair with a fresh self-signed one. The new pair is
/// generated and written beside the old one before anything is replaced, so
/// a failed generation leaves the old pair in place. A certificate that is
/// not self-signed (one a CA issued) is only replaced with `--force`.
#[cfg(feature = "http-transport")]
pub fn handle_cert_renew_command(force: bool) -> Result<()> {
    use skrills_server::tls_auto::generate_self_signed_cert;

    let tls_path = tls_dir()?;
    let cert_path = tls_path.join("cert.pem");

    if cert_path.exists() && !force {
        let cert_info = parse_cert_info(&cert_path)?;
        if !cert_info.is_self_signed {
            bail!(
                "{} is not self-signed; renewing would replace it with a self-signed \
                 certificate. Renew it with its issuer, or pass --force to replace it anyway.",
                cert_path.display()
            );
        }
        if let Some(days) = cert_info.days_until_expiry {
            if cert_info.is_valid && days > CERT_EXPIRY_WARNING_DAYS {
                println!(
                    "Certificate is still valid for {} days. Use --force to renew anyway.",
                    days
                );
                return Ok(());
            }
        }
    }

    let (cert_pem, key_pem) = generate_self_signed_cert()?;
    install_pair(&tls_path, cert_pem.as_bytes(), Some(key_pem.as_bytes()))?;

    let key_path = tls_path.join("key.pem");
    println!("Certificate renewed successfully!");
    println!("  Certificate: {}", cert_path.display());
    println!("  Private Key Fingerprint: {}", key_fingerprint(&key_path)?);

    Ok(())
}

#[cfg(not(feature = "http-transport"))]
pub fn handle_cert_renew_command(_force: bool) -> Result<()> {
    bail!("Certificate renewal requires the 'http-transport' feature")
}

/// Whether `data` starts with a PEM certificate header.
fn is_pem_certificate(data: &[u8]) -> bool {
    String::from_utf8_lossy(data)
        .trim_start()
        .starts_with("-----BEGIN CERTIFICATE-----")
}

/// Whether `data` looks like a PEM private key.
fn is_pem_private_key(data: &[u8]) -> bool {
    let text = String::from_utf8_lossy(data);
    let text = text.trim_start();
    text.starts_with("-----BEGIN ") && text.contains("PRIVATE KEY-----")
}

/// Checks that `key_pem` is the private key for the certificate in
/// `cert_pem`, by comparing the certificate's public key with the one the
/// key derives. A PEM header alone said nothing about either.
#[cfg(feature = "http-transport")]
fn check_key_matches_cert(cert_pem: &[u8], key_pem: &[u8]) -> Result<()> {
    use rcgen::PublicKeyData;
    use x509_parser::prelude::*;

    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem)
        .map_err(|e| anyhow::anyhow!("Failed to parse certificate PEM: {e}"))?;
    let (_, cert) = X509Certificate::from_der(&pem.contents)
        .map_err(|e| anyhow::anyhow!("Failed to parse X.509 certificate: {e}"))?;
    let key_text = std::str::from_utf8(key_pem).context("Key file is not UTF-8 PEM")?;
    let key = rcgen::KeyPair::from_pem(key_text).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse the private key to check it against the certificate: {e} \
             (a PKCS#1 RSA key can be converted with `openssl pkcs8 -topk8 -nocrypt`)"
        )
    })?;
    if cert.public_key().subject_public_key.data.as_ref() != key.der_bytes() {
        bail!("The private key does not match the certificate's public key");
    }
    Ok(())
}

/// Handle `skrills cert install <path>` command.
///
/// The certificate and the key are both checked before anything is written,
/// then installed together, so a typo in `--key` leaves the installed pair
/// untouched instead of pairing a new certificate with the old key.
pub fn handle_cert_install_command(
    cert_source: PathBuf,
    key_source: Option<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let tls_path = tls_dir()?;
    let cert_dest = tls_path.join("cert.pem");
    let key_dest = tls_path.join("key.pem");

    if !cert_source.exists() {
        bail!("Certificate file not found: {}", cert_source.display());
    }
    // The bytes checked are the bytes installed: re-reading the file could
    // see different content.
    let cert_data = read_limited(&cert_source)?;
    if !is_pem_certificate(&cert_data) {
        bail!(
            "Certificate file does not appear to be valid PEM format: {}",
            cert_source.display()
        );
    }

    let key_data = match key_source {
        Some(ref key_src) => {
            if !key_src.exists() {
                bail!("Key file not found: {}", key_src.display());
            }
            let data = read_limited(key_src)?;
            if !is_pem_private_key(&data) {
                bail!(
                    "Key file does not appear to be a PEM private key: {}",
                    key_src.display()
                );
            }
            Some(data)
        }
        None => None,
    };

    // A certificate is useless with the wrong key: check the given key, or,
    // without `--key`, the key already installed that it would be paired with.
    #[cfg(feature = "http-transport")]
    match key_data.as_deref() {
        Some(key) => check_key_matches_cert(&cert_data, key)?,
        None if key_dest.exists() => {
            let installed = read_limited(&key_dest)?;
            check_key_matches_cert(&cert_data, &installed).with_context(|| {
                format!(
                    "The certificate does not fit the installed key {}; pass its key with --key",
                    key_dest.display()
                )
            })?;
        }
        None => bail!("No key is installed for this certificate; pass it with --key"),
    }

    install_pair(&tls_path, &cert_data, key_data.as_deref())?;

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

/// One-line status of the certificate at `cert_path`, for server startup.
///
/// Takes the certificate the server will actually use (the `--tls-cert`
/// path, or the auto-generated one) rather than always reporting
/// `~/.skrills/tls/cert.pem`.
#[cfg(feature = "http-transport")]
pub fn get_cert_status_summary(cert_path: &Path) -> Option<String> {
    if !cert_path.exists() {
        return None;
    }

    let cert_info = match parse_cert_info(cert_path) {
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
    let status = if !cert_info.is_valid {
        "NOT VALID"
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
        "TLS: {} ({} days until expiry [{}]{})",
        cert_path.display(),
        days,
        status,
        self_signed
    ))
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
        use skrills_server::tls_auto::generate_self_signed_cert;

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
    fn pem_certificate_check_accepts_a_certificate_header() {
        assert!(is_pem_certificate(
            b"  -----BEGIN CERTIFICATE-----\nMIIBxTCCAW...\n-----END CERTIFICATE-----\n"
        ));
    }

    #[test]
    fn pem_certificate_check_rejects_other_text() {
        assert!(!is_pem_certificate(b"this is not a PEM file"));
    }

    /// A certificate and its key. With `http-transport` the install parses
    /// both and checks they match, so the pair has to be real.
    #[cfg(feature = "http-transport")]
    fn test_pair() -> (String, String) {
        skrills_server::tls_auto::generate_self_signed_cert().unwrap()
    }

    #[cfg(not(feature = "http-transport"))]
    fn test_pair() -> (String, String) {
        (
            "-----BEGIN CERTIFICATE-----\nMIIBxTCCAW...\n-----END CERTIFICATE-----\n".into(),
            "-----BEGIN PRIVATE KEY-----\nMIIEvQIBAD...\n-----END PRIVATE KEY-----\n".into(),
        )
    }

    /// A temp HOME with the TLS directory resolved under it.
    /// Fields drop in order: HOME is restored before the lock is released.
    struct Home {
        _home: skrills_test_utils::EnvVarGuard,
        dir: tempfile::TempDir,
        _env: std::sync::MutexGuard<'static, ()>,
    }

    impl Home {
        fn new() -> Self {
            let env = skrills_test_utils::env_guard();
            let dir = tempfile::tempdir().unwrap();
            let home = skrills_test_utils::set_env_var("HOME", Some(dir.path().to_str().unwrap()));
            Self {
                _home: home,
                dir,
                _env: env,
            }
        }

        fn tls(&self) -> PathBuf {
            self.dir.path().join(".skrills/tls")
        }

        fn source(&self, name: &str, content: &str) -> PathBuf {
            let path = self.dir.path().join(name);
            fs::write(&path, content).unwrap();
            path
        }
    }

    #[test]
    fn validity_compares_timestamps_not_whole_days() {
        let now = 1_000_000;
        // SA-27: 23 hours left was reported expired.
        assert_eq!(validity_at(now - 10, now + 23 * 3600, now), (true, 0));
        // SA-27: a certificate not yet valid was reported valid.
        assert!(!validity_at(now + 3600, now + 90 * 86_400, now).0);
        assert_eq!(validity_at(now - 10, now - 3600, now), (false, -1));
        assert_eq!(validity_at(now - 10, now + 40 * 86_400, now), (true, 40));
    }

    /// SA-49: the old tests re-implemented the install; this drives it.
    #[test]
    fn install_writes_cert_and_owner_only_key() {
        let home = Home::new();
        let (cert_pem, key_pem) = test_pair();
        let cert = home.source("c.pem", &cert_pem);
        let key = home.source("k.pem", &key_pem);

        handle_cert_install_command(cert, Some(key), OutputFormat::Json).unwrap();

        assert_eq!(
            fs::read_to_string(home.tls().join("cert.pem")).unwrap(),
            cert_pem
        );
        assert_eq!(
            fs::read_to_string(home.tls().join("key.pem")).unwrap(),
            key_pem
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(home.tls().join("key.pem")), 0o600);
            assert_eq!(mode(home.tls()), 0o700);
        }
    }

    /// SA-26: `.mode(0o600)` only applied on creation, so an existing
    /// world-readable key.pem stayed world-readable.
    #[test]
    #[cfg(unix)]
    fn install_tightens_a_previously_readable_key() {
        use std::os::unix::fs::PermissionsExt;
        let home = Home::new();
        fs::create_dir_all(home.tls()).unwrap();
        let old_key = home.tls().join("key.pem");
        fs::write(&old_key, "old").unwrap();
        fs::set_permissions(&old_key, fs::Permissions::from_mode(0o644)).unwrap();
        let (cert_pem, key_pem) = test_pair();
        let cert = home.source("c.pem", &cert_pem);
        let key = home.source("k.pem", &key_pem);

        handle_cert_install_command(cert, Some(key), OutputFormat::Text).unwrap();

        let mode = fs::metadata(&old_key).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    /// SA-11: a missing or invalid key used to fail after cert.pem was
    /// already replaced.
    #[test]
    fn install_with_a_bad_key_leaves_the_installed_pair_alone() {
        let home = Home::new();
        fs::create_dir_all(home.tls()).unwrap();
        fs::write(home.tls().join("cert.pem"), "old cert").unwrap();
        fs::write(home.tls().join("key.pem"), "old key").unwrap();
        let cert = home.source("c.pem", &test_pair().0);
        let not_a_key = home.source("k.pem", "hello");

        let missing = handle_cert_install_command(
            cert.clone(),
            Some(home.dir.path().join("typo.pem")),
            OutputFormat::Text,
        );
        let invalid = handle_cert_install_command(cert, Some(not_a_key), OutputFormat::Text);

        assert!(missing
            .unwrap_err()
            .to_string()
            .contains("Key file not found"));
        assert!(invalid.unwrap_err().to_string().contains("private key"));
        assert_eq!(
            fs::read_to_string(home.tls().join("cert.pem")).unwrap(),
            "old cert"
        );
        assert_eq!(
            fs::read_to_string(home.tls().join("key.pem")).unwrap(),
            "old key"
        );
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn install_refuses_a_key_that_does_not_match_the_certificate() {
        let home = Home::new();
        let (cert_pem, _) = test_pair();
        let (_, other_key) = test_pair();
        let cert = home.source("c.pem", &cert_pem);
        let key = home.source("k.pem", &other_key);

        let err = handle_cert_install_command(cert, Some(key), OutputFormat::Text)
            .unwrap_err()
            .to_string();

        assert!(err.contains("does not match"), "{err}");
        assert!(!home.tls().join("cert.pem").exists());
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn install_without_a_key_refuses_a_certificate_the_installed_key_does_not_match() {
        let home = Home::new();
        let (old_cert, old_key) = test_pair();
        fs::create_dir_all(home.tls()).unwrap();
        fs::write(home.tls().join("cert.pem"), &old_cert).unwrap();
        fs::write(home.tls().join("key.pem"), &old_key).unwrap();
        let new_cert = home.source("c.pem", &test_pair().0);

        let err = handle_cert_install_command(new_cert, None, OutputFormat::Text)
            .unwrap_err()
            .to_string();

        assert!(err.contains("--key"), "{err}");
        assert_eq!(
            fs::read_to_string(home.tls().join("cert.pem")).unwrap(),
            old_cert
        );
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn install_refuses_a_certificate_that_only_has_a_pem_header() {
        let home = Home::new();
        let cert = home.source(
            "c.pem",
            "-----BEGIN CERTIFICATE-----\nMIIBxTCCAW...\n-----END CERTIFICATE-----\n",
        );
        let key = home.source("k.pem", &test_pair().1);

        assert!(handle_cert_install_command(cert, Some(key), OutputFormat::Text).is_err());
        assert!(!home.tls().join("cert.pem").exists());
    }

    /// A failed certificate rename used to leave the new key beside the old
    /// certificate, and the temp files behind.
    #[test]
    fn install_puts_the_old_key_back_when_the_certificate_cannot_be_installed() {
        let home = Home::new();
        fs::create_dir_all(home.tls().join("cert.pem/blocker")).unwrap();
        fs::write(home.tls().join("key.pem"), "old key").unwrap();
        let (cert_pem, key_pem) = test_pair();

        let result = install_pair(&home.tls(), cert_pem.as_bytes(), Some(key_pem.as_bytes()));

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(home.tls().join("key.pem")).unwrap(),
            "old key"
        );
        let leftovers: Vec<_> = fs::read_dir(home.tls())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "cert.pem" && n != "key.pem")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// `/dev/zero` reports length 0, so the metadata check let it be read
    /// without limit.
    #[cfg(unix)]
    #[test]
    fn read_limited_refuses_an_endless_special_file() {
        let err = read_limited(Path::new("/dev/zero"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("10 MB"), "{err}");
    }

    #[test]
    fn status_reports_an_unreadable_key_instead_of_aborting() {
        let home = Home::new();
        fs::create_dir_all(home.tls().join("key.pem")).unwrap();

        handle_cert_status_command(OutputFormat::Text).unwrap();
        let (exists, fingerprint, error) = key_status(&home.tls().join("key.pem"));
        assert!(exists);
        assert!(fingerprint.is_none());
        assert!(error.is_some());
    }

    /// SA-10: renewal deleted a CA-issued certificate and replaced it with a
    /// self-signed one.
    #[test]
    #[cfg(feature = "http-transport")]
    fn renew_refuses_a_ca_issued_certificate_without_force() {
        use rcgen::{CertificateParams, DnType, Issuer, KeyPair};

        let ca_key = KeyPair::generate().unwrap();
        let mut ca = CertificateParams::default();
        ca.distinguished_name.push(DnType::CommonName, "test CA");
        let issuer = Issuer::new(ca, ca_key);
        let leaf_key = KeyPair::generate().unwrap();
        let mut leaf = CertificateParams::default();
        leaf.distinguished_name.push(DnType::CommonName, "leaf");
        let now = time::OffsetDateTime::now_utc();
        leaf.not_before = now - time::Duration::days(1);
        leaf.not_after = now + time::Duration::days(5);
        let leaf_pem = leaf.signed_by(&leaf_key, &issuer).unwrap().pem();

        let home = Home::new();
        fs::create_dir_all(home.tls()).unwrap();
        fs::write(home.tls().join("cert.pem"), &leaf_pem).unwrap();
        fs::write(home.tls().join("key.pem"), "ca key").unwrap();

        let err = handle_cert_renew_command(false).unwrap_err().to_string();

        assert!(err.contains("not self-signed"), "{err}");
        assert_eq!(
            fs::read_to_string(home.tls().join("cert.pem")).unwrap(),
            leaf_pem
        );
        assert_eq!(
            fs::read_to_string(home.tls().join("key.pem")).unwrap(),
            "ca key"
        );
    }

    #[test]
    #[cfg(feature = "http-transport")]
    fn renew_with_force_replaces_the_pair() {
        let home = Home::new();
        fs::create_dir_all(home.tls()).unwrap();
        fs::write(home.tls().join("cert.pem"), "old cert").unwrap();
        fs::write(home.tls().join("key.pem"), "old key").unwrap();

        handle_cert_renew_command(true).unwrap();

        let info = parse_cert_info(&home.tls().join("cert.pem")).unwrap();
        assert!(info.is_valid && info.is_self_signed);
        assert!(is_pem_private_key(
            &fs::read(home.tls().join("key.pem")).unwrap()
        ));
        assert!(!home.tls().join("cert.pem.tmp").exists());
    }

    /// SA-28: the summary names the certificate it describes.
    #[test]
    #[cfg(feature = "http-transport")]
    fn status_summary_reports_the_given_certificate() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("custom.pem");
        let (cert_pem, _) = skrills_server::tls_auto::generate_self_signed_cert().unwrap();
        fs::write(&path, cert_pem).unwrap();

        let summary = get_cert_status_summary(&path).unwrap();

        assert!(
            summary.contains("custom.pem") && summary.contains("[OK]"),
            "{summary}"
        );
        assert!(get_cert_status_summary(&tmp.path().join("absent.pem")).is_none());
    }
}
