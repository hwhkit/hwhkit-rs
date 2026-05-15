# Security policy

## Reporting a vulnerability

If you believe you have found a security vulnerability in any
`hwhkit-*` crate, **please do not open a public GitHub issue.**
Public reports give attackers a head start on services that have not
yet upgraded.

Instead, email the maintainer:

  **louishwh@gmail.com**

with subject `hwhkit security`. Include:

1. The affected crate(s) and version(s).
2. A description of the issue, including impact assessment.
3. Reproduction steps or a proof-of-concept, if possible.
4. Your name / handle if you would like to be credited in the advisory.

You should receive an acknowledgement within **3 business days**.
We will work with you on a fix and a coordinated disclosure timeline.

## Supported versions

While `hwhkit` is pre-1.0 (the `0.x` line), only the **latest
published minor version** receives security fixes. Pin a recent
version in `Cargo.toml` to stay on the supported track.

Once 1.0 ships, the most recent two minor versions will be supported
in parallel.

## Scope

The following components are security-sensitive and treated with
higher priority:

| Component                          | Why it's sensitive |
| ---------------------------------- | ------------------ |
| `hwhkit_core::jwt`                 | Token verification chain (JWKS, multi-algorithm) |
| `hwhkit::production::rate_limit`   | DoS protection — bypass = service-level outage |
| `hwhkit::production::idempotency`  | Body-fingerprint cache; weakness → request replay |
| `hwhkit::production::circuit_breaker` | Outbound HTTP gate; weakness → cascading failure |
| `hwhkit-config` (remote patch)     | Config injection surface |
| `hwhkit-integration-*` (URL parsers, credential handling) | Connection-string parsing, credential redaction |

Bugs in non-security code are tracked through normal GitHub issues.

## Out of scope

- Vulnerabilities in the underlying SDK crates (`sqlx`, `redis`,
  `aws-sdk-s3`, etc.) — please report those to the upstream project.
  We will patch our pin once upstream releases a fix.
- Denial-of-service via misconfiguration (e.g. setting
  `max_connections = 1_000_000`). Operators are responsible for
  sensible configuration; we provide validation but not omniscient
  guard-rails.
- Issues that require an attacker to already have root on the host
  running the service.
