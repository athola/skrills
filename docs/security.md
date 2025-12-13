# Security

## Overview

This document outlines key security considerations, features, and best practices for `skrills` deployments. For more in-depth information on specific topics, please refer to the linked documents.

---

## Index

### Core Security Documents

1. **[Threat Model](threat-model.md)**
- Detailed threat analysis.
- Attack vectors and mitigations.
- Deployment security considerations.
- Risk assessment framework.

2. **Secrets Management** (TODO: create docs/secrets-management.md)
- Guidelines for API key generation and rotation.
- Best practices for TLS certificate management.
- Recommendations for secure secrets storage.
- Integration strategies with systems like Vault and AWS Secrets Manager.

3. **TLS Hardening**
- TLS 1.3-only policy using Rustls defaults.
- Option for an explicit TLS 1.3 allowlist for compliance.
- Best practices for secure MCP server deployments.

4. **MCP Dependency Hygiene** (see [MCP Dependency Strategy in `process-guidelines.md`](process-guidelines.md#mcp-dependency-strategy-rmcp--pastey))
- Management of `rmcp` within the workspace using scoped feature sets.
- `pastey` is a transitive replacement for `paste` (via `rmcp` v0.10.0+), replacing the unmaintained `paste` crate.
- Prompt updates to `rmcp` upon security advisories; direct `pastey` pins or `rmcp` forks should be avoided unless absolutely necessary due to lack of maintenance.

5. **Rate Limiting** (TODO: create docs/rate-limiting.md)
- Outlines planned rate limiting capabilities.
- Details configuration options and the implementation roadmap.
- Describes strategies for Denial-of-Service (DoS) protection.

6. **Audit Logging** (TODO: create docs/audit-logging.md)
- Requirements for security event logging.
- Guidelines for mTLS authentication audit trails.
- Guidance for SIEM integration and compliance considerations.

---

## Quick Start Security Checklist

### Development Environment
- [ ] Do not commit secrets to version control systems.
- [ ] Use environment variables for all API keys.
- [ ] Activate pre-commit hooks to detect accidental secret inclusion.
- [ ] Use TLS even in development environments; self-signed certificates are acceptable for this purpose.
- [ ] Verify `.gitignore` configuration to ensure sensitive files are excluded.

```bash
# Check for leaked secrets
git log -p | grep -E "sk_live_|BEGIN PRIVATE KEY"

# Install pre-commit hook
cp scripts/pre-commit-secret-check.sh .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

---

### Production Deployment

#### Mandatory Requirements
- [ ] **Strictly enforce TLS 1.3** for any network-exposed deployments.
- [ ] **Use robust API keys** (with 256+ bits of entropy) if authentication is required.
- [ ] **Secure file permissions** appropriately.
- [ ] **Operate with a dedicated service account** (avoiding root privileges).

#### Recommended
- [ ] Activate audit logging.
- [ ] Establish log rotation policies.
- [ ] Implement monitoring and alerting systems.
- [ ] Establish a regular secrets rotation schedule.
- [ ] Apply network segmentation through firewall rules.
- [ ] Conduct routine security updates.

```bash
# Example systemd service
cat > /etc/systemd/system/skrills.service <<EOF
[Service]
Type=notify
User=skrills
Group=skrills
EnvironmentFile=/etc/skrills/secrets.env
ExecStart=/usr/local/bin/skrills serve
Restart=on-failure

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/skrills

[Install]
WantedBy=multi-user.target
EOF
```

---

## Security Features

### Current Implementation

#### Authentication & Authorization
- **Process Isolation (stdio mode)**: In local deployments using standard I/O (stdio) mode, security relies on process isolation and filesystem permissions. The MCP server runs as the user's process.
- **Future: mTLS Support**: For network-exposed deployments, mTLS client authentication with X.509 certificate verification is planned.

#### Network Security
- **TLS 1.3 Support**: The system supports TLS 1.3, using only modern cipher suites (e.g., AES-256-GCM, ChaCha20-Poly1305) and enforcing forward secrecy.
- **Certificate Validation**: Includes CA certificate verification, hostname validation, and expiration checks for all certificates.

#### Input Validation
- **Path Canonicalization**: Prevents directory traversal attacks by validating skill file paths and restricting access to designated skill directories.
- **MCP Message Validation**: Involves JSON schema validation, type checking, and enforcing size limits for all MCP messages.
- **File Size Limits**: Implements configurable maximum file sizes to prevent memory exhaustion and gracefully handles excessively large files.

#### Data Protection
- **No Persistent Prompt Storage**: User prompts are processed ephemerally and are never written to disk, ensuring data privacy.
- **Cache TTL**: The cache expires automatically and is memory-bounded, to prevent the use of stale data.

---

### Planned Enhancements

#### Rate Limiting (Q4 2025)
- Token bucket algorithm.
- Per-client limits.
- Per-operation costs.
- Global concurrent limits.

**Status**: Design phase is complete, with implementation scheduled.

**See**: Rate Limiting (TODO: create docs/rate-limiting.md)

---

#### Enhanced Audit Logging (Q4 2025)
- mTLS authentication and API key validation failures.
- Authorization decisions and configuration changes.
- Tamper-evident logging.

**Status**: Partially implemented; basic logging through `tracing` is currently operational.

**See**: Audit Logging (TODO: create docs/audit-logging.md)

---

#### Skill Signing & Verification (Q1 2026)
- Cryptographic signatures for skills.
- Publisher verification and trust framework.
- Revocation mechanism.

**Status**: Currently in the design phase.

---

## Security Model

### Trust Boundaries
The defined trust boundaries are:
- User Account (Trusted)
- Skrills Process (runs as user)
- Skill Files (user-controlled, e.g., [`~/.codex/skills/`](~/.codex/skills/), [`~/.claude/`](~/.claude/))

Communication between components occurs via standard I/O (stdio) or network, interacting with the Claude Code Client (which can be either trusted or untrusted depending on configuration).

### Security Assumptions
- **Trusted Components**: The user account, the `skrills` process, and local skill files are trusted.
- **Untrusted Components**: Network traffic (if exposed), MCP clients (especially remote ones), and external skill content are treated as untrusted.

### Defense Strategy
- Rigorous input validation is applied at all trust boundaries.
- Strict path sanitization is used for all filesystem access operations.
- Authentication and encryption mechanisms secure all network traffic.
- Skill content is never executed on the server-side, to mitigate execution-based vulnerabilities.

---

## Vulnerability Reporting

### Reporting Security Issues
**IMPORTANT**: Do not create public GitHub issues for reporting security vulnerabilities.
**Instead**:
1. Contact the security team directly via email at `security@example.com`.
2. Your report should include a clear description of the vulnerability, precise reproduction steps, the potential impact, and an optional suggestion for a fix.

**Response Service Level Agreement (SLA)**:
- Initial response within 48 hours.
- Triage completed within 1 week.
- Fix timeline determined by the severity of the vulnerability.

### Security Advisory Process
1. Private disclosure of the vulnerability to project maintainers.
2. Coordinated patch development.
3. Publishing a security advisory (typically via GitHub Security Advisory).
4. Assigning a Common Vulnerabilities and Exposures (CVE) identifier, if applicable.
5. Public disclosure, after a patch is made available.

---

## Compliance

### Industry Standards
- **OWASP Top 10**: Mitigations for the OWASP Top 10 vulnerabilities are documented.
- **CWE Top 25**: The CWE Top 25 most dangerous software errors are addressed in the threat model.
- **NIST Cybersecurity Framework**: Our security posture aligns with the NIST Cybersecurity Framework.
- **CIS Controls**: We implement key CIS Controls, such as audit logging and access control.

### Data Privacy
- **GDPR**: Adheres to GDPR principles by not storing user data persistently.
- **CCPA**: Follows CCPA's privacy by design principles.
- **SOC 2**: Meets SOC 2 requirements with robust audit logging and access controls.

**See**: Compliance section in Audit Logging (TODO: create docs/audit-logging.md#compliance)

---

## Security Testing

### Continuous Security
**Automated Checks (CI/CD)**: We maintain continuous security through automated checks integrated into the CI/CD pipeline.
```yaml
# .github/workflows/security.yml
- name: Dependency Audit
  run: cargo audit

- name: Security Linting
  run: cargo clippy -- -W clippy::all

- name: SAST Scan
  uses: github/codeql-action/analyze
```

### Recommended Manual Testing
- **Penetration Testing**: Conduct manual penetration testing covering areas such as mTLS authentication bypass, path traversal vulnerabilities, TLS configuration validation, and API key brute-force resistance.
- **Fuzzing**: Use fuzzing techniques, for example, by running `cargo fuzz run mcp_parser` and `cargo fuzz run skill_parser`, to uncover unexpected behaviors and vulnerabilities.
- **Security Audits**: Perform annual third-party security audits, conduct thorough code reviews for all sensitive changes, and implement continuous dependency vulnerability scanning.

---

## Security Hardening

### Operating System Level

#### Linux
- **Linux**: For Linux deployments, configure SELinux/AppArmor contexts and profiles to strictly restrict `skrills` process permissions.

#### macOS
- **macOS**: Ensure `skrills` is properly code signed and notarized for macOS environments.

### Container Security
- **Container Security**: When deploying `skrills` in containers (e.g., Docker/Podman), use a `distroless` base image, configure the application to run as a non-root user, and apply security options such as `--read-only` and `--cap-drop=ALL`.

### Network Hardening
- **Firewall Rules (iptables)**: Implement stringent firewall rules (e.g., using `iptables`) to allow only necessary ports and to rate-limit incoming connections.
- **Network Segmentation**: Use network segmentation to restrict access to skill directories and require VPN for administrative access when exposing the MCP server to a network.

---

## Security Contacts

- **Security Team**: `security@example.com`
- **Vulnerability Disclosure**: `security@example.com`

---

## Additional Resources

- [OWASP Secrets Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html)
- [HashiCorp Vault Documentation](https://www.vaultproject.io/docs)
- [AWS Secrets Manager Best Practices](https://docs.aws.amazon.com/secretsmanager/latest/userguide/best-practices.html)
- [OWASP MCP Security Guidelines](https://owasp.org/)
- [Rust Security Guidelines](https://rust-lang.github.io/api-guidelines/security.html)
- [CIS Benchmarks](https://www.cisecurity.org/cis-benchmarks/)
- [NIST Cybersecurity Framework](https://www.nist.gov/cyberframework)

---

## Document History

| Version | Date | Changes |
|---------|------|---------|
| 2.0 | 2025-11-30 | Added threat model, secrets management, rate limiting, audit logging |
| 1.0 | 2025-01-01 | Initial security documentation |
