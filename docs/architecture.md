# Architecture

This document describes the high-level architecture of TPT AV Commissioning. It maps to the repository layout in §6 of `spec.txt`.

## Principles

- **Test and prove, don't operate.** The application's job is Model / Discover / Test / Measure / Compare / Diagnose / Document / Prove (§56). Operating, routing, triggering, automating, and synchronising responsibilities belong to `tpt-av-control`.
- **Domain separation over crate count.** The exact number of crates may be reduced; the boundaries between the domain model, drivers, test model, and reporting must stay clean (§6).
- **Even split between core engine and presentation.** The CLI and the Tauri desktop UI share the same core engine (§34).
- **Offline-first.** A fully offline install must support project creation, configuration, manual and local/network tests, reports, baselines, defects, evidence, and sign-off (§45).

## Crate map

| Crate | Responsibility | Tracks to |
|---|---|---|
| `core` | Application services: persistence (SQLite), projection, orchestrating tests against drivers, evidence and baseline stores | §25–27, engine |
| `model` | Pure domain model: `Project`, `Room`, `Device`, `Endpoint`, `Connection`, `SignalGraph`, path requirements | §7–9 |
| `device` | Device-level primitives shared by drivers: identity, state, commands, responses, capabilities | §10, §39 |
| `driver` | SDK traits (`DeviceDriver`, `DeviceDiscovery`, `DeviceCommand`, `DeviceState`, `DeviceCapability`) and driver plumbing | §10, §39 |
| `test` | Test model and schema: `CommissioningTest`, `TestStatus`, `TestResult`, requirement types | §12–13, §16 |
| `runner` | Dependency-aware scheduling, parallelism, timeouts, retries, cancellation, device locking, result persistence | §14, §17–18 |
| `report` | Report rendering (PDF/HTML/CSV/JSON), sign-off, handover package | §30–32 |
| `cli` | Command-line interface sharing the core engine | §34 |
| `tauri` | Desktop UI (dashboard, system graph, device view, test runner, results, defects) | §28 |
| `testkit` | Mock devices, mock networks, fault injection | §46.2, §47 |

## Layering

```
                    ┌────────────────────────────┐
                    │  tauri (desktop UI)  / cli │
                    └───────────┬────────────────┘
                                │
                    ┌───────────▼────────────────┐
                    │        core (services)     │
                    │  persistence · projection  │
                    └───────┬───────────┬────────┘
                            │           │
               ┌────────────▼─┐   ┌─────▼────────────┐
               │   runner     │   │     report       │
               └──────┬───────┘   └──────────────────┘
                      │
        ┌─────────────▼──────────┐
        │      test (model)      │
        └─────────────┬──────────┘
                      │
┌─────────────┐  ┌────▼─────────────────┐  ┌─────────────┐
│    model    │◄─┤       driver         │──►│   device    │
│ (domain)    │  │ (SDK + protocols)    │  │ (primitives)│
└─────────────┘  └─────────────────────┘  └─────────────┘
```

Dependencies flow inward: `core` depends on `runner`, `test`, `driver`, `model`; `runner` and `report` depend on `test` and `model`; `driver` depends on `device` and `model`; `model` has no internal dependencies.

## Concurrency model

- Asynchronous device communication throughout (§44). Non-blocking UI thread.
- The runner executes independent tests in parallel with bounded concurrency and cancellation support.
- Device mutation is guarded by `DeviceLock` to prevent concurrent mutation races (§18).

## Data flow

1. A `Project` (plus rooms, devices, endpoints, connections) is loaded from the SQLite store and/or the portable YAML manifest.
2. Devices are matched to drivers via the driver registry using identity/protocol matching.
3. Test suites run through the runner, which schedules tests, acquires device locks, executes through drivers, and records `TestResult`s.
4. Results, measurements, defects, baselines, and evidence metadata are persisted to SQLite; binary evidence lives in the managed project asset directory.
5. Reports and handover packages are generated from the persisted run state.