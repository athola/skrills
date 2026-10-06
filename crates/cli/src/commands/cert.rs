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
    let home = dirs::home_dir().context("Could not determine home directory")?;
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
fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata: {}", path.display()))?;
    if meta.len() > MAX_CERT_FILE_SIZE {
        bail!("{} exceeds the maximum size of 10 MB", path.display());
    }
    fs::read(path).with_context(|| format!("Failed to read {}", path.display()))
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
/// key.pem that was previously world-readable.
fn install_pair(tls_path: &Path, cert_pem: &[u8], key_pem: Option<&[u8]>) -> Result<()> {
    create_tls_dir(tls_path)?;
    let cert_dest = tls_path.join("cert.pem");
    let key_dest = tls_path.join("key.pem");

    let cert_tmp = write_temp(&cert_dest, cert_pem, false)?;
    let key_tmp = match key_pem.map(|k| write_temp(&key_dest, k, true)).transpose() {
        Ok(k) => k,
        Err(e) => {
            let _ = fs::remove_file(&cert_tmp);
            return Err(e);
        }
    };

    if let Some(key_tmp) = key_tmp {
        fs::rename(&key_tmp, &key_dest)
            .with_context(|| format!("Failed to install key at {}", key_dest.display()))?;
    }
    fs::rename(&cert_tmp, &cert_dest)
        .with_context(|| format!("Failed to install certificate at {}", cert_dest.display()))?;
    Ok(())
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
        let fingerprint = key_fingerprint(&key_path)?;
        println!("Private Key: FOUND");
        println!("  Fingerprint: {}", fingerprint);
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

/// Validate that a file contains PEM-formatted certificate data.
///
/// Checks that the file starts with the standard PEM certificate header.
/// Returns `Ok(true)` if valid, `Ok(false)` if not valid PEM format.
pub fn validate_pem_format(path: &PathBuf) -> Result<bool> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file for PEM validation: {}", path.display()))?;
    Ok(content
        .trim_start()
        .starts_with("-----BEGIN CERTIFICATE-----"))
}

/// Whether `data` looks like a PEM private key.
fn is_pem_private_key(data: &[u8]) -> bool {
    let text = String::from_utf8_lossy(data);
    let text = text.trim_start();
    text.starts_with("-----BEGIN ") && text.contains("PRIVATE KEY-----")
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
    let cert_data = read_limited(&cert_source)?;
    match validate_pem_format(&cert_source) {
        Ok(true) => {}
        Ok(false) => bail!(
            "Certificate file does not appear to be valid PEM format: {}",
            cert_source.display()
        ),
        Err(e) => bail!(
            "Could not validate certificate file {}: {}",
            cert_source.display(),
            e
        ),
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

    const CERT_PEM: &str =
        "-----BEGIN CERTIFICATE-----\nMIIBxTCCAW...\n-----END CERTIFICATE-----\n";
    const KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBAD...\n-----END PRIVATE KEY-----\n";

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
        let cert = home.source("c.pem", CERT_PEM);
        let key = home.source("k.pem", KEY_PEM);

        handle_cert_install_command(cert, Some(key), OutputFormat::Json).unwrap();

        assert_eq!(
            fs::read_to_string(home.tls().join("cert.pem")).unwrap(),
            CERT_PEM
        );
        assert_eq!(
            fs::read_to_string(home.tls().join("key.pem")).unwrap(),
            KEY_PEM
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
        let cert = home.source("c.pem", CERT_PEM);
        let key = home.source("k.pem", KEY_PEM);

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
        let cert = home.source("c.pem", CERT_PEM);
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
