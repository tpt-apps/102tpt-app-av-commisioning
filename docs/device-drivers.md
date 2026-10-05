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

Implemented in the `tpt-app-av-commissioning-profile` crate (§41). Profiles let integrators and vendors describe a device without writing code. A worked example lives in `drivers/examples/osc-projector.yaml`.

```yaml
schema_version: 1
device:
  id: osc-projector
  match:
    manufacturer: OscCo
    model: Beam-1              # or a list of models
  protocol:
    type: osc                  # osc | midi | tcp | udp | http | websocket | serial | snmp
    port: 9000
    timeout_ms: 400
  commands:                    # power_on, power_off, power_cycle, set_input, freeze,
    power_on:  { send: /power, args: [true] }     # generate_test_pattern, set_audio_volume,
    set_input: { send: /input, args: ["$input"] } # set_audio_mute, set_route, read_edid, measure_latency
  state:
    power: { query: /power }
```

Rules (enforced by `DeviceProfile::from_yaml_str`):

1. **Versioned** — `schema_version` is mandatory; an unsupported version is reported as such (not as an unrelated parse error).
2. **No arbitrary code execution** — profiles are declarative data. Custom YAML tags, unknown fields, unparseable input and oversized documents (256 KiB) are rejected. Command names come from a closed list.
3. **Placeholders** (`"$input"`, `$pattern`, `$level_db`, `$muted`, `$frozen`, `$source`, `$destination`) are filled from the command being run, and each is valid in exactly one command.
4. **Matching** — a profile applies only to a device whose reported manufacturer *and* model match (`DeviceProfile::matches`, trimmed and case-insensitive). A device reporting neither never matches.
5. Bounded: ≤ 64 commands / state fields, ≤ 16 args per command, timeouts 1 ms–30 s, no credentials.

A driver turns a profile plus a host into a configured driver; `OscDriverConfig::from_profile` does this for OSC and refuses profiles of any other protocol. Only OSC is implemented so far; other protocol types parse and validate but have no driver yet.

## Safety

- Strict protocol parsing with bounded network reads.
- Configurable timeouts and scan ranges for every network operation.
- Authentication support where device protocols require it; credentials are stored encrypted (or via the OS credential store) and never written into project manifests or profiles.