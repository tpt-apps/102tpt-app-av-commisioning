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
- `midi/` — MIDI 1.0 (MIDI 2.0/UMP not yet)
- `network/` — network-focused protocols/discovery
- `examples/` — worked driver examples and profile samples

`tpt-av-control` is the device-control foundation for OSC/MIDI/MIDI 2.0/DMX/Art-Net/sACN/WebRTC/control surfaces; generic improvements should be contributed back upstream where applicable.

### Driver matrix

| `protocol.type` | Crate | Commands | State | Notes |
|---|---|---|---|---|
| `osc` | `drivers/osc` | OSC message (address + args) | query address, typed reply | UDP; no acknowledgement |
| `tcp` | `drivers/generic` | text line, optional `ack` | query line + parser | one connection per operation |
| `udp` | `drivers/generic` | one datagram, optional `ack` | query datagram + parser | `terminator: none` by default |
| `serial` | `drivers/generic` | text line, optional `ack` | query line + parser | real port paths only (`COMn`, `/dev/...`) |
| `websocket` | `drivers/generic` | text message, optional `ack` | query message + `extract` | `ws://` only |
| `http` | `drivers/generic` | `METHOD /path` + JSON `body` | `GET /path` + `extract` | `http://` only; credentials supplied by the caller |
| `snmp` | `drivers/generic` | none (read-only) | OID per field | v1/v2c GET; missing OIDs are not reported |
| `midi` | `drivers/midi` | fixed channel message (`cc 1 20 127`) | last message the device sent | no SysEx or raw MIDI, by design |

Cross-cutting rules for every driver: explicit unicast targets (no broadcast, multicast or unspecified), every wait bounded by the timeout, received data size-capped, malformed replies are errors never guesses, substituted values cannot inject a second command (line breaks refused; URL- and JSON-escaped for HTTP), `Arbitrary` commands are refused where raw input would be unsafe (HTTP, SNMP, MIDI), and credentials never live in profiles. A driver reports a field the device did not return as *unreported*, which tests surface as `Inconclusive`.

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

Text protocols (`type: tcp`) add: `protocol.terminator` (`lf`/`crlf`/`cr`, default `crlf`); per command `ack` (the reply line that acknowledges it; absent = fire-and-forget) and `$placeholders` embedded in `send` (`"INPT $input"`; booleans render as `1`/`0`); per state field `parser` (`auto`, `bool`, `int`, `float`, `text`, `power_state`) and `strip_prefix`. Substituted values must be short, printable and single-line, so a value can never smuggle a second command to the device. See `drivers/examples/tcp-projector.yaml`.

A driver turns a profile plus a host into a configured driver; `OscDriverConfig::from_profile` does this for OSC and refuses profiles of any other protocol. OSC (`drivers/osc`) and TCP (`drivers/generic`) are implemented; other protocol types parse and validate but have no driver yet. TCP opens one connection per operation (stateless, tolerant of devices that drop idle connections).

## Discovery

Implemented in `drivers/network` (`tpt-app-av-commissioning-discovery`), configured by `DiscoveryConfig` in the driver SDK (§11, §36). Discovery finds *candidates*; adding one to a project is the engineer's decision, and `match_profiles` only pairs a candidate with a profile when the device itself reported a matching manufacturer and model.

| Protocol | Scope | Behaviour |
|---|---|---|
| `tcp_probe` | host / CIDR | connect to each listed port and close; sends nothing; optional banner read (128 bytes, printable ASCII) |
| `snmp` | host / CIDR | SNMPv2c GET of sysDescr, sysObjectID, sysName; community supplied by the caller |
| `osc` | host / CIDR | one argument-less OSC query (default `/info`); any OSC reply counts |
| `mdns` | multicast + interface | PTR query with the unicast-response bit; we never join the group or bind 5353 |
| `ssdp` | multicast + interface | M-SEARCH; replies grouped by sender; `LOCATION` recorded, never fetched |
| `midi` / `serial` / `audio` | local | enumerate this machine's ports and devices |

Rules enforced by `DiscoveryConfig::validate` *before anything is sent*: at least one protocol and each must fit the scope; unicast scans cover at most `max_hosts` (default 256, hard limit 4096) IPv4 hosts; ranges outside private, link-local, CGNAT and loopback need `allow_public`; CIDR must be a network address (a typo is an error, not a silent widening); at most 16 ports per probe; timeouts ≤ 10 s; concurrency ≤ 64; multicast requires a named interface that exists on this machine. Everything a device sends back is bounded (datagram, header and record limits), parsed strictly, and reduced to printable text before it is stored. A failing mechanism is reported in `DiscoveryReport::errors` without discarding what the others found, and a pass can be cancelled.

## Safety

- Strict protocol parsing with bounded network reads.
- Configurable timeouts and scan ranges for every network operation.
- Authentication support where device protocols require it; credentials are stored encrypted (or via the OS credential store) and never written into project manifests or profiles.