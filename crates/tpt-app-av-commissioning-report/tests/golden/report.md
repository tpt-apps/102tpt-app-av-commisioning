# TPT AV Commissioning Report

**Project:** Boardroom Fitout (`prj-boardroom-01`)
**Client:** Meridian Business <Group>
**Site:** Level 12, 88 Collins St
**Generated:** 2026-01-15T18:02:00+00:00
**Software:** 0.1.0

Rooms: 2 · Devices: 7 · Connections: 11

## Summary

| Total | Pass | Fail | Warning | Blocked | Skipped | Manual | Inconclusive |
|---|---|---|---|---|---|---|---|
| 4 | 1 | 1 | 0 | 0 | 0 | 2 | 0 |

## Results

| Test | Status | Messages |
|---|---|---|
| display.identity | pass | identity matches: ACME Beamer 9000 |
| display.signal | fail | no signal on hdmi2; resolution read 0x0 |
| projector.image-quality | manual |  |
| pa.speech-level | manual | measured level -18 dBFS |

## Defects

- `DEF-001` [investigating] Display does not lock signal on input 3

## Sign-off

**A. Engineer** — PASS WITH ACCEPTED EXCEPTIONS

Exceptions:
- DEF-001 accepted: input 3 unused in final layout
