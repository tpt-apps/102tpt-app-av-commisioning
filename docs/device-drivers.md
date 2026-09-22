# Device Drivers

This document describes the driver SDK (§10, §39–41 of `spec.txt`) and the versioned device profile format. Implemented in the `driver` and `device` crates.

## Guiding rule

Device-specific communication must not leak into test logic. A test talks to a `DeviceDriver`; a driver knows how to talk to a device.

## Public traits

```rust
trait DeviceDriver {
    fn identity(&self) -> DeviceIdentity;
    fn discover(&self) -> Result<DeviceState>;
    fn get_state(&self) -> Result<DeviceState>;
    fn execute(&self, command: DeviceCommand) -> Result<DeviceResponse>;
    fn capabilities(&self) -> DeviceCapabilities;
}
```

| Trait | Role |
|---|---|
| `DeviceDriver` | Main interface between the runner and a physical device |
| `DeviceDiscovery` | Find devices on the network / bus (§11) |
| `DeviceCommand` | A typed command a driver can execute |
| `DeviceState` | The state a driver can read back |
| `DeviceCapability` | A single known capability flag |

`DeviceCapabilities` is a struct of flags:

| Capability | Description |
|---|---|
| `can_power_on` | Driver can power the device on |
| `can_power_off` | Driver can power the device off |
| `can_read_state` | Driver can read current state |
| `can_select_input` | Driver can select an input |
| `can_generate_test_pattern` | Driver can produce a test pattern |
| `can_read_signal_status` | Driver can report signal lock/format |
| `can_read_edid` | Driver can read the EDID |
| `can_measure_latency` | Driver can measure signal latency |

Drivers may communicate over OSC, MIDI, serial, TCP/UDP, HTTP, WebSocket, SNMP, vendor-proprietary protocols, or any combination. Where protocol behaviour requires it, drivers are async.

## Registry and matching

- Drivers declare `protocol`, `manufacturer`, `model` matching, `discovery method`, `capabilities`, and profile reference.
- The application matches project devices to drivers by identity and protocol config.
- Drivers are compiled into the application initially; there is no dynamic plugin system yet (§39). A dynamic plugin system is only evaluated once the static architecture is proven.

## Generic protocol drivers

Generic protocol drivers live under `drivers/`:

- `generic/` — TCP, UDP, HTTP, WebSocket, serial, SNMP (where appropriate)
- `osc/` — OSC
- `midi/` — MIDI (incl. MIDI 2.0 groundwork)
- `network/` — network-focused protocols/discovery
- `examples/` — worked driver examples and profile samples

`tpt-av-control` is the device-control foundation for OSC/MIDI/MIDI 2.0/DMX/Art-Net/sACN/WebRTC/control surfaces; generic improvements should be contributed back upstream where applicable.

## Device profile format

The versioned device profile format (`drivers/examples/`) lets integrators and vendors describe devices without writing code:

```yaml
profile:
  schema_version: 1
  manufacturer: Example
  models:
    - Example-5000
    - Example-5010
protocol:
  transport: tcp           # or osc, midi, serial, http, websocket, ...
  port: 4352
commands:
  - id: power_on
    match: { opcode: "0x01" }
  - id: read_state
    request: { opcode: "0x03" }
    state_query: { field: "power", parser: "bool" }
```

Rules:

1. **Versioned** — `schema_version` is mandatory and validated.
2. **No arbitrary code execution** — profiles are declarative data; they cannot contain or reference executable code. Validation rejects executable content.
3. Profiles must match on `manufacturer`/`model` before they may be applied to a device.

## Safety

- Strict protocol parsing with bounded network reads.
- Configurable timeouts and scan ranges for every network operation.
- Authentication support where device protocols require it; credentials are stored encrypted (or via the OS credential store) and never written into project manifests or profiles.