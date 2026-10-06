# Project Format

This document describes project persistence, the human-readable manifest, and configuration snapshots (§25–27 of `spec.txt`).

## Storage

Projects use **SQLite** for execution history and a human-readable **YAML manifest** for the portable system definition.

```
Project/
├── project.sqlite
├── project.yaml          # portable manifest (system definition)
├── assets/
│   ├── screenshots/
│   ├── recordings/
│   ├── configurations/
│   └── evidence/
├── reports/
└── exports/
```

Binary evidence is stored as immutable project assets and referenced from results — never embedded in the database.

## SQLite schema

Persisted tables:

- `projects` (metadata)
- `rooms`
- `devices`
- `endpoints`
- `connections`
- `test_suites`
- `test_definitions`
- `test_executions`
- `results`
- `measurements`
- `defects` (with a `payload` column carrying the full defect record)
- `evidence` (metadata only)
- `configuration_baselines` (with a `payload` column carrying the full
  baseline snapshot; older databases are migrated on open)
- `audit_log` (§33 event trail)

## Manifest format (§26)

The manifest captures the portable system definition and is version-controllable:

```yaml
project:
  schema_version: 1
  name: "Client Boardroom"

rooms:
  - id: boardroom
    name: "Boardroom"

devices:
  - id: projector-01
    type: projector
    manufacturer: Example
    model: Example-5000
    address: 192.168.1.40
```

Rules:

- `schema_version` is mandatory and validated.
- Ids are human-readable and stable across edits.
- No arbitrary executable code may be embedded in the manifest.
- Credentials are never committed into project manifests.
- Imported configuration files are parsed safely.

## Configuration snapshots (§27)

Where device protocols permit, configuration is captured and stored under `assets/configurations/`.

Snapshots are always labelled:

- `before-commissioning`
- `after-commissioning`
- `before-maintenance`
- `after-maintenance`

A stored snapshot is **never silently overwritten** — creating a snapshot with an existing label fails or requires explicit confirmation and produces a new revision.