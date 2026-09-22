# Report Format

This document describes reporting and sign-off (§30–32 of `spec.txt`). Implemented in the `report` crate.

## Outputs

| Format | Contents |
|---|---|
| PDF | Cover page, project details, system summary, device inventory, topology, test summary, detailed results, defects, evidence, engineer sign-off, software/profile version, date/time |
| HTML | Same content, browsable |
| CSV | Tabular results/measurements/defects for integrator pipelines |
| JSON | Machine-readable results for automation |

## Honesty rules

A report must never imply measurement that did not occur:

- The report distinguishes **automated test**, **manual inspection**, and **engineer acceptance** outcomes.
- Every result records its execution mode (from the test model).
- Reports record the software version, driver/profile versions, and date/time for reproducibility.

## Sign-off (§32)

- Engineer name, date, and a result that includes the option **"PASS WITH ACCEPTED EXCEPTIONS"** plus the exception list.
- Signing a report records an audit event (`audit-log`).

## Handover package (§30)

A handover package bundles: the commissioning report, device inventory, network inventory, signal topology, test results, failed/resolved test list, defect register, configuration snapshots, evidence, the test profile, software version, and the project baseline.

## Verification

Report generation is unit-tested, and golden reports (project + suite + mock device state → stable expected report) guard against regressions (§46.4).