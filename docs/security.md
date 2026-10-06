# Security & Network Safety

This document records the security and network-safety posture (§36, §38 of `spec.txt`).

## Execution policy

Projects carry an `execution_policy` with explicit allow flags, default-denied:

- `allow_power_cycle`
- `allow_configuration_changes`
- `allow_network_changes`

Destructive operations (power cycles, reboots, configuration writes, network changes) require the corresponding flag **and** explicit operator confirmation.

The runner enforces the policy at dispatch: a test whose mutation kind is not permitted is blocked (never failed) before it can touch a device, and dependents follow the normal blocked-not-failed rule.

## Network safety

- All network operations require explicit interface/target selection — no implicit probing of every interface.
- Configurable timeouts and scan ranges; bounded network reads; strict protocol parsing.
- No aggressive/arbitrary scanning by default (§36; see discovery §11).
- Dry-run mode is available so operators can see precisely what a run would do.
- Command logging records what was sent and to which device.

## No remote code execution

- Project files, test DSL, and device profiles are declarative data. Arbitrary executable code in them is explicitly disallowed and rejected at validation time.
- Drivers are compiled into the application; there is no dynamic plugin/loading surface yet (§39).

## Credentials

- Authentication supported for device protocols that require it.
- Credentials stored encrypted, with OS credential-store integration where practical — never in project manifests, test files, or profiles.
- Sensitive values are excluded from reports by default.

## Access control and audit

- Project access permissions are delegated to the OS file permissions.
- A tamper-evident audit log records notable actions (§33): project created, device added/modified, test run, result changed, defect created/closed, baseline created, configuration imported, report generated, report signed.

## Local API

The optional local API binds only to `127.0.0.1` by default and never to all interfaces. If external binding is ever enabled, authentication becomes mandatory (§35).