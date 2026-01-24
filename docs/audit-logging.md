# Audit Logging

## Overview

Audit logging provides visibility into security-relevant events, supports compliance requirements, and enables incident response and forensic analysis. This document defines the audit logging standards for `skrills` deployments.

Basic logging is currently implemented via the `tracing` crate.

---

## Security Event Logging

### Event Categories

Audit logs capture the following security-relevant events:

| Category | Events | Priority |
|----------|--------|----------|
| **Authentication** | mTLS handshake success/failure, API key validation, certificate expiration warnings | Critical |
| **Authorization** | Access denied events, permission checks, resource access attempts | High |
| **Configuration** | Settings changes, skill directory modifications, cache configuration updates | High |
| **Skill Operations** | Skill discovery, skill loading, skill rendering, cache hits/misses | Medium |
| **System Events** | Server start/stop, graceful shutdown, error conditions | Medium |

### Event Structure

Each audit log entry includes:

```json
{
  "timestamp": "2025-01-07T12:00:00.000Z",
  "event_id": "uuid-v4",
  "event_type": "authentication.mtls.failure",
  "severity": "warning",
  "source": {
    "component": "mcp_server",
    "version": "<current_version>"
  },
  "actor": {
    "type": "client",
    "identifier": "client-cert-cn",
    "ip_address": "192.168.1.100"
  },
  "action": {
    "operation": "authenticate",
    "resource": "/mcp/v1/tools",
    "outcome": "failure",
    "reason": "certificate_expired"
  },
  "context": {
    "request_id": "uuid-v4",
    "correlation_id": "uuid-v4"
  }
}
```

### Severity Levels

| Level | Description | Examples |
|-------|-------------|----------|
| **Critical** | Security breach or imminent threat | Authentication bypass, unauthorized access |
| **Error** | Security control failure | Certificate validation error, key rotation failure |
| **Warning** | Potential security issue | Multiple failed auth attempts, certificate near expiry |
| **Info** | Normal security operations | Successful authentication, configuration change |
| **Debug** | Detailed diagnostic data | Full request/response for troubleshooting |

---

## mTLS Authentication Audit Trails

### Certificate Lifecycle Events

Track these certificate-related events for mTLS deployments:

1. **Certificate Presentation**: Receipt of client certificate, chain validation initiation, and CA trust verification.
2. **Validation Outcomes**: Success (logging CN, serial, expiry), specific failure reasons, and revocation checks.
3. **Session Events**: TLS session establishment, resumption, and termination.

### Example mTLS Audit Log

```json
{
  "timestamp": "2025-01-07T12:00:00.000Z",
  "event_type": "authentication.mtls.success",
  "severity": "info",
  "actor": {
    "certificate": {
      "subject_cn": "claude-code-client",
      "issuer_cn": "skrills-ca",
      "serial": "1234567890ABCDEF",
      "not_after": "2026-01-07T00:00:00Z",
      "fingerprint_sha256": "AB:CD:EF:..."
    },
    "ip_address": "192.168.1.100"
  },
  "action": {
    "operation": "mtls_handshake",
    "outcome": "success",
    "tls_version": "TLSv1.3",
    "cipher_suite": "TLS_AES_256_GCM_SHA384"
  }
}
```

### Authentication Failure Tracking

Monitor authentication anomalies such as failed attempts per client, geographic anomalies, time-based patterns, and certificate irregularities.

---

## SIEM Integration

### Log Formats

Logs are emitted in standard formats for SIEM ingestion:

| Format | Use Case | Configuration |
|--------|----------|---------------|
| **JSON Lines** | General SIEM ingestion | Default structured output |
| **CEF** | ArcSight, QRadar | `SKRILLS_LOG_FORMAT=cef` |
| **LEEF** | IBM QRadar | `SKRILLS_LOG_FORMAT=leef` |
| **Syslog** | Traditional SIEM | `SKRILLS_LOG_FORMAT=syslog` |

### Alerting

SIEM alerts should cover:

| Alert | Condition | Severity |
|-------|-----------|----------|
| **Brute Force Detection** | > 5 auth failures from same source in 5 minutes | High |
| **Certificate Expiry** | Certificate expires within 30 days | Medium |
| **Unauthorized Access** | Any authorization denied event | High |
| **Configuration Change** | Any configuration modification | Medium |
| **Service Disruption** | Server restart or error spike | High |

---

## Compliance

### GDPR Considerations

Audit logging adheres to GDPR via:

1. **Data Minimization**: Logging only necessary security data, avoiding full user prompts, and using pseudonymization.
2. **Retention Limits**: Enforcing retention periods (typically 90 days for operational logs, 1 year for security events) with automated rotation.
3. **Access Controls**: Restricting and logging access to audit logs.
4. **Data Subject Rights**: Documenting personal data in logs and supporting access/erasure requests.

### SOC 2 Requirements

SOC 2 Type II compliance requires demonstrable controls:

- **CC6.1/6.2/6.3**: Logging authentication, authorization decisions, and access removal.
- **CC7.1/7.2/7.3**: Real-time monitoring, anomaly alerting, and searchable audit trails for incident response.

### Log Integrity

Tamper-evident logging ensures compliance through immutable storage (append-only/WORM), cryptographic integrity (log signing/hash chaining), and strict access auditing.

---

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SKRILLS_AUDIT_ENABLED` | Enable audit logging | `false` |
| `SKRILLS_AUDIT_LEVEL` | Minimum severity to log | `info` |
| `SKRILLS_AUDIT_FORMAT` | Output format (json, cef, syslog) | `json` |
| `SKRILLS_AUDIT_OUTPUT` | Output destination (stdout, file, syslog) | `stdout` |
| `SKRILLS_AUDIT_FILE` | Path to audit log file | `/var/log/skrills/audit.log` |
| `SKRILLS_AUDIT_RETENTION_DAYS` | Log retention period | `90` |

### Example Configuration

```toml
# skrills.toml
[audit]
enabled = true
level = "info"
format = "json"
output = "file"
file = "/var/log/skrills/audit.log"
retention_days = 90

[audit.events]
authentication = true
authorization = true
configuration = true
skill_operations = false  # Disable for high-volume environments
```

---

## Related Documents

- [Security Overview](security.md)
- [Threat Model](threat-model.md)

---

## References

- [OWASP Logging Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)
- [NIST SP 800-92](https://csrc.nist.gov/publications/detail/sp/800-92/final)
- [SOC 2 Trust Service Criteria](https://www.aicpa.org/interestareas/frc/assuranceadvisoryservices/sorhome)
- [CIS Controls](https://www.cisecurity.org/controls)

---

## Document History

| Version | Date | Changes |
|---------|------|---------|
| 1.0 | 2025-01-07 | Initial audit logging documentation |
