# Security Considerations

This chapter summarizes security guidance from [`docs/security.md`](docs/security.md) and [`docs/threat-model.md`](docs/threat-model.md) into an actionable playbook. It explains assets to protect, common attack methods, and critical configurations for a secure `skrills` deployment.

## Assets Protected

- **High-Value Data**: Includes sensitive information like user prompts, configuration files, and skill content.
- **Availability Assets**: Core components vital for service continuity, including the MCP server process, the cache system, and filesystem skill roots.

## Threat Model

Key Attackers:
- **Malicious Skills**: Untrusted `SKRILL.md` content used to inject harmful instructions into prompts.
- **Supply-Chain Compromises**: Vulnerabilities in compromised dependencies.
- **Local Attackers**: Users with filesystem access who try to tamper with skill files or configuration.

Primary Mitigations:
- Path canonicalization and skill-root allowlists to prevent directory traversal attacks.
- Strict size guards (e.g., `--max-bytes`) to limit the impact of malicious content.
- Future skill signing and allowlisting to reduce the skill injection attack surface.

## Production Checklist

1. **Operate as an unprivileged service account**; configure strict file permissions (`chmod 600`) for sensitive configuration files.
2. **Enable audit logging** (once available) and review logs regularly.
3. **Restrict skill sources**: Only load skills from trusted directories.
4. **Monitor skill content**: Review skill files for suspicious content before use.

## Secrets Management

- Store secrets securely in environment variables or integrate with a dedicated secrets manager; never commit them to version control.
- Apply `chmod 600` permissions to configuration files containing sensitive data.
- Plan for regular credential rotations where applicable.

## Deployment Patterns

- **Stdio/Local Mode**: Relies on process isolation and filesystem permissions. Store secrets in environment files, not directly within manifests.
- **Claude Code Integration**: The hook-based integration inherits Claude Code's security model. Ensure Claude Code itself is properly configured.

## Future Security Enhancements

- **Tamper-Evident Audit Logs**: Tamper-evident audit log channel for skill loading events.
- **Skill Signing/Allowlisting**: Skill signing and allowlisting to harden the supply chain for `SKILL.md` content.
