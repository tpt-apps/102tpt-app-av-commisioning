# Domain Model

This document describes the core domain model (§7–9 of `spec.txt`), implemented in the `model` crate.

## Project

A `Project` is the root aggregate: the thing being commissioned.

- id, name, client (optional), site (optional)
- references rooms, devices, connections, test suites

## Room

A named space within a project (Boardroom, Training Room 1, Auditorium, Lecture Theatre 4, Meeting Room 203).

- id, name, description (optional)
- references its devices and connections

## Device

A physical piece of AV equipment that can send, receive, control, or measure a signal.

- id, name, manufacturer, model, serial number, firmware, device type
- references its endpoints and addresses

`DeviceType` is extensible. Initial categories: Display, Projector, Camera, Microphone, Speaker, Amplifier, DSP, Switcher, Matrix, Scaler, Encoder, Decoder, MediaServer, ControlProcessor, TouchPanel, LightingController, NetworkDevice, Computer, Other.

The distinction between a device and its endpoints matters because a device may have many independently testable endpoints.

## Endpoint

Something that can send, receive, control, or measure a signal.

- id, name, kind, device reference

`EndpointKind`: VideoInput, VideoOutput, AudioInput, AudioOutput, Network, Control, Gpio, Usb, Serial, Lighting, Clock.

## DeviceAddress

How a device is reached: an opaque, extensible address representation covering network addresses (host + port), serial ports, OSC/MIDI addressing, and arbitrary addressing schemes to be described by drivers.

## Connection

A known relationship between two endpoints.

- id, source endpoint, destination endpoint, signal type, transport, expected outcome

`SignalType`: Video, Audio, AudioVideo, Network, Control, Lighting, Clock, Unknown.

`Transport` is extensible for proprietary/emerging protocols: HDMI, SDI, DisplayPort, USB, AES3, Analog, Dante, NDI, RTP, RTSP, OSC, MIDI, ArtNet, SACN, Ethernet, Serial, Other.

A `ConnectionExpectation` describes what "working" means for that connection (signal present, endpoint reachable, resolution, audio channels, latency budget…).

## Signal path

A path is a route from a source through intermediate devices to a destination:

```
Source A → Matrix 1 in 3 → Matrix 1 out 7 → Scaler 2 → Projector 1
```

The path is a **graph**, not a simple list (`SignalGraph` with nodes and edges), so a single device can participate in multiple paths and failures can be located precisely.

Each path can carry requirements:

```yaml
path:
  source: presentation-pc
  destination: projector-1
requirements:
  video:
    resolution: 3840x2160
    frame_rate: 60
    hdr: false
  audio:
    channels: 2
  latency:
    max_ms: 100
```

## Identity

All aggregate ids (`ProjectId`, `RoomId`, `DeviceId`, `EndpointId`, `ConnectionId`, `TestSuiteId`) are newtype-wrapped strings that are stable, human-readable in project manifests (e.g. `projector-01`), and version-controllable. Uniqueness within a project is required.

## Validation rules

- A device must belong to exactly one room.
- A connection's endpoints must belong to devices in the same room.
- A connection's source kind must be compatible with its signal type and transport (e.g. a VideoInput source is not valid for an HDMI video connection).
- All references in a project must resolve within that project.

## Defect (§23)

A failed test converts into a `Defect` (severity Critical / Major / Minor /
Cosmetic; status Open → Investigating → Fixed → Retest Required → Verified,
with Accepted and Deferred as explicit exits) referencing the tests that
found it and the evidence captured. `Verified`, `Accepted` and `Deferred`
are terminal; a failed retest moves back to `Retest Required` so nothing is
silently reported as fixed.

## Baseline & drift (§21–22)

A `Baseline` snapshots device identity fingerprints (manufacturer, model,
serial, firmware, addresses plus tracked configuration attributes), signal
routes, and the run's per-test statuses; its fingerprint is a SHA-256 over
that content. Comparing a later run highlights only meaningful changes:
worse (or better) test outcomes, new failures, and tests that no longer
produce a verdict. Comparing current state detects drift: firmware changes,
address changes, replaced units (different serial — never assumed identical
because the model matches), attribute changes, and route changes.

## Audit log (§33)

Significant project actions (project created, devices added/modified, test
runs, result changes, defect lifecycle, baselines, configuration imports,
report generation and signing) are recorded as `AuditEvent`s with
timestamp, actor (`User(name)` or `System`), object id, and structured
details.