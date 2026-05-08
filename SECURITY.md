# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in chainwatch-rs, please report it responsibly:

1. **Do not** open a public GitHub issue for security vulnerabilities.
2. Email your report to **security@yablokolabs.com** with:
   - A description of the vulnerability
   - Steps to reproduce
   - Potential impact assessment
   - Suggested fix (if any)

We will acknowledge receipt within 48 hours and aim to provide an initial assessment within 5 business days.

## Scope

The following are in scope for security reports:

- Authentication/authorization bypasses
- SQL injection or query manipulation
- Information disclosure (e.g., internal error details leaking)
- Denial of service via resource exhaustion
- Dependency vulnerabilities with a known exploit path
- Container escape or privilege escalation

## Out of Scope

- Rate limiting bypass (informational only)
- Issues requiring physical access
- Social engineering

## Disclosure Policy

We follow coordinated disclosure. After a fix is released, we will:

1. Credit the reporter (unless anonymity is requested)
2. Publish a security advisory via GitHub Security Advisories
3. Release a patched version
