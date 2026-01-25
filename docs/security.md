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

6. **[Audit Logging](audit-logging.md)**
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
In local deployments using standard I/O (stdio) mode, security relies on process isolation and filesystem permissions, where the MCP server runs as the user's process. For network-exposed deployments, we plan to implement mTLS client authentication with X.509 certificate verification.

#### Network Security
The system supports TLS 1.3, prioritizing modern cipher suites like AES-256-GCM and ChaCha20-Poly1305 to enforce forward secrecy. It validates CA certificates, hostnames, and expiration dates for all connections.

#### Input Validation
We prevent directory traversal attacks by validating skill file paths and restricting access to designated skill directories. All MCP messages undergo JSON schema validation, type checking, and size limit enforcement. Configurable file size limits further prevent memory exhaustion and handle large files gracefully.

#### Data Protection
User prompts are processed ephemerally and never written to disk. The cache is memory-bounded and expires automatically to prevent the use of stale data.

---

### Planned Enhancements

#### Rate Limiting
- Token bucket algorithm.
- Per-client limits.
- Per-operation costs.
- Global concurrent limits.

**Status**: Design phase is complete, with implementation scheduled.

**See**: Rate Limiting (TODO: create docs/rate-limiting.md)

---

#### Enhanced Audit Logging
- mTLS authentication and API key validation failures.
- Authorization decisions and configuration changes.
- Tamper-evident logging.

**Status**: Partially implemented; basic logging through `tracing` is currently operational.

**See**: [Audit Logging](audit-logging.md)

---

#### Skill Signing & Verification
- Cryptographic signatures for skills.
- Publisher verification and trust framework.
- Revocation mechanism.

**Status**: Currently in the design phase.

---

## Security Model

### Trust Boundaries
The primary trust boundaries lie between the user account (trusted), the Skrills process running as that user, and the skill files they control. Communication channels like standard I/O or the network, as well as the MCP clients themselves (especially remote ones), are treated as untrusted boundaries.

### Security Assumptions
We assume the user account, the `skrills` process, and local skill files are trusted entities. Conversely, we treat all network traffic and external skill content as untrusted, requiring strict validation.

### Defense Strategy
We apply rigorous input validation at all trust boundaries and strict path sanitization for filesystem access. Network traffic is secured via authentication and encryption, and skill content is never executed on the server-side to mitigate execution-based vulnerabilities.

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
- **OWASP Top 10**: Mitigations for Top 10 vulnerabilities are documented.
- **CWE Top 25**: The Threat Model addresses the 25 most dangerous software errors.
- **NIST Cybersecurity Framework**: Security posture aligns with NIST CSF.
- **CIS Controls**: Implements key controls such as audit logging and access control.

### Data Privacy
- **GDPR**: Skrills does not store user data persistently, aligning with GDPR principles.
- **CCPA**: Follows privacy-by-design principles.
- **SOC 2**: Supports SOC 2 requirements via robust audit logging and access controls.

**See**: [Compliance section in Audit Logging](audit-logging.md#compliance)

---

## Security Testing

### Continuous Security
**Automated Checks (CI/CD)**: Security checks run in the CI/CD pipeline.
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
- **Penetration Testing**: Conduct manual penetration testing covering mTLS authentication bypass, path traversal vulnerabilities, TLS configuration validation, and API key brute-force resistance.
- **Fuzzing**: Use fuzzing techniques (e.g., `cargo fuzz run mcp_parser`) to uncover unexpected behaviors.
- **Security Audits**: Perform annual third-party security audits, conduct code reviews for sensitive changes, and implement continuous dependency scanning.

---

## Security Hardening

### Operating System Level

#### Linux
Configure SELinux/AppArmor contexts and profiles to strictly restrict `skrills` process permissions.

#### macOS
Ensure `skrills` is properly code signed and notarized.

### Container Security
When deploying in containers (e.g., Docker/Podman), use a `distroless` base image, configure the application to run as a non-root user, and apply security options such as `--read-only` and `--cap-drop=ALL`.

### Network Hardening
- **Firewall Rules (iptables)**: Implement stringent firewall rules to allow only necessary ports and rate-limit incoming connections.
- **Network Segmentation**: Restrict access to skill directories and require VPN for administrative access when exposing the MCP server.

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
