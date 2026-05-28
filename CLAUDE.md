# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project purpose

HorusTechWatch is a **read-only state publisher** for a Companytec Horustech fuel-pump automation device (gas station concentrator). It polls the device over TCP and publishes a single `state.json` to a Windows share; a separate third-party tool fetches that file and decides what to alarm on. The device is live and actively managing fuel dispensers; nothing in this codebase may disturb its operation.

**Status:** deployed and running in production since 2026-05-27 on Windows host `PST-ZAM-04`, publishing to `\\Servidor\pista\Relátório-pista\modulo_alarmes\concentrador\state.json`.

## Device

- **IP**: `192.168.25.91`
- **Network**: WiFi (same /24 subnet as the dev machine at `192.168.25.114`)
- **Ping latency**: variable 10–132ms — socket reads must use a **5-second timeout**

### Port status (verified 2026-05-26)

| Port | Status | Use |
|------|--------|-----|
| **857** | **CLOSED** — needs enabling via HRS Console | Intended for secondary apps (our permanent target) |
| 2001 | OPEN | Main POS software |
| 1771 | OPEN | Companytec configuration tools |

**Two-phase port plan:**
- **Phase 1 (dev/test):** use port **2001** — safe because the device handles multiple concurrent TCP connections and all our commands are read-only.
- **Phase 2 (permanent deployment):** enable port 857 via HRS Console, update config to use it.

## Hardcoded safety constraints

These are not configurable and must be enforced in code regardless of what any config file says:

- **Poll interval floor: 30 seconds** — reject any configured value below this
- **Default poll interval: 60 seconds**
- **Socket timeout: 5 seconds** per read
- **Send-side allowlist**: the function that writes to the socket must validate the command index against `{0x01, 0x0B, 0x12, 0x1B, 0x25}` and throw/panic on anything else — even in tests

## Horustech communication protocol (DT214 Rev.14)

### Frame structure

```
>PCCCCX...KK
```

- `>` — delimiter (not included in checksum calculation)
- `P` — `?` for query (PC → device), `!` for response (device → PC)
- `CCCC` — 4-char hex: byte-length of the DATA field
- `X...` — data: 2-char hex command index + parameters
- `KK` — 2-char hex checksum

**Checksum**: sum ASCII values of every character after `>` (i.e. from `P` to the last data byte), drop the most-significant byte, format as 2 uppercase hex digits.

Example query:
```
>?000201KK   →  data = "01" (2 bytes), CCCC = "0002"
checksum covers: ?000201
```

### Health-check commands (read-only — safe to use)

All five are pure queries. They cause zero side effects on the device.

| Command | Index | Query frame | What it returns |
|---------|-------|-------------|-----------------|
| **Status** | `01` (0x01) | `>?00020162` | One character per configured nozzle: `B`=blocked, `L`=free, `A`=fueling, `F`=fault, `E`=waiting, `P`=ready, `#`=busy, `!`=generic error |
| **Device info** | `12` (0x12) | `>?00021264` | Firmware version, battery level (`0`=normal `1`=low `2`=critical), MAC, IP, serial number |
| **Clock** | `0B` (0x0B) | `>?00020B73` | Device date/time (year, month, day, hour, minute, second) |
| **Diagnostics** | `1B` (0x1B) | `>?00041B5AEC` | Per-pump status: `R`=responding, `F`=fault, `N`=not configured |
| **Wireless diag** | `25` (0x25) | `>?00022568` | Per-pump: status (`R`/`F`/`N`), LQI [0..F], RSSI [0..F] |

Response frames start with `>!` and follow the same `CCCCX...KK` structure. The command index echoes back as the first two data bytes.

### Device info (0x12) response field layout

Full response template (up to 182 bytes):
```
>!CCCC12 vVV.VV fFF.FF DD/MM/AA B bbbbb E eeee C-NNNNNNNNN DD/MM/AA DD/MM/AA MM:MM:MM:MM:MM:MM III.III.III.III;DD/MM/AA dflt CCCCCCCC;FIIIHDPT;RRR.RRR.RRR.RRR;ppppp;a;...KK
```

Key fields (positions are space/separator delimited, not fixed byte offsets):

| Field | Width | Description | Alarm? |
|-------|-------|-------------|--------|
| `v` + `V[05]` | 1+5 | Boot-loader descriptor + version (e.g. `B01.00`) | no |
| `f` + `F[05]` | 1+5 | Firmware descriptor + version (e.g. `F08.03`) | no |
| `DD/MM/AA` | 8 | Firmware date | no |
| **`B[01]`** | **1** | **Battery level: `0`=normal, `1`=low, `2`=critical** | **yes** |
| `b[05]` | 5 | Battery voltage, e.g. `12,84` (12.84 V) | no |
| `E[01]` | 1 | External network: `0`=off, `1`=low, `2`=normal, `3`=high | maybe |
| `e[04]` | 4 | External network voltage | no |
| `C-N[08]` | 10 | Permissions char + `-` + 8-digit serial number | no |
| `DD/MM/AA` | 8 | Manufacturing date | no |
| `DD/MM/AA` | 8 | Last valid date | no |
| `M[17]` | 17 | MAC address (`XX:XX:XX:XX:XX:XX`) | no |
| `I[15]` | 15 | IP address zero-padded (`192.168.025.091`) | no |
| `d[01]` (after `;date `) | 1 | IP type: `D`=DHCP, `F`=Fixed | no |
| `f[01]` | 1 | Active protocol: `C`=Companytec, `c`=CBC, `P`=PAN, `D`=disabled | no |
| **`D[01]`** (in `FIIIHDPT`) | **1** | **Battery hardware status: `D`=present+charging, `L`=present+not charging, `F`=absent, `i`=inverted** | **yes** |
| `H[01]` | 1 | Automation type: `H`=Horustech, `M`=H4 | no |

**Live device reading (2026-05-26):**
```
>!009E12B01.00 F08.03 22/07/19 0 12,84 2 0113 3-00010427 17/01/17 26/05/26 00:26:28:11:04:27 192.168.025.091;00/00/00 Fc  00000000;c900HDNN;000.000.000.000;00000;D;FB
```
Battery level `B` = `0` (normal), voltage = 12.84 V, hardware status `D` = present and charging.

### Alarm thresholds to watch

- Any nozzle status becomes `F` (fault) or `!` (generic error)
- Battery level `1` = warning, `2` = critical
- Diagnostic pump status `F` = fault
- Device clock drift > threshold (device clock vs local clock)
- No response / socket timeout = device unreachable

### Commands that must NEVER be used

Do not send any write/control command. Dangerous indices include:

- `06` — Increment (advances the fuel delivery read pointer — permanent side effect on the device)
- `02` — fuel delivery read variants that interact with the pointer
- `0D`/`0E`/`17` — identifier write/delete/auto-record
- `0A` — calendar set
- `1A` — configuration write
- `27` — diagnostic mode toggle (some sub-modes are destructive)
- `28`/`2A`/`2C`/`2E` — pump management (preset, price change, mode change)
- `30` — blacklist modification

### Protocol reference

Full manual: `companytec_kit_desenvolvimento/concentradores/Manuais/DT214 - Protocolo de Comunicação Horustech.pdf` (48 pages, Rev.14, 10/12/2025)

## Architecture

Each poll cycle:
1. Probe-write the output directory. If unreachable → skip the device poll, append an `output_unreachable` event to the local audit log, sleep until next interval.
2. Open a TCP socket to `device.ip:device.port` with a 5-second connect timeout. No keep-alive — one connection per cycle.
3. Run the five allowlisted queries serially (status, clock, device_info, diagnostics, wireless) with a 5-second read timeout each. Single in-flight enforced by the type of the function.
4. Update the in-memory `StateAccumulator`. Successful commands advance `last_success_at` + `last_success`; failures preserve the previous good values so consumers see how stale each command is.
5. Atomically publish `state.json` (write to `state.json.tmp`, fsync, rename).
6. Append a one-line JSONL audit entry (`poll_ok` / `poll_partial` / `connect_failed` / `parse_failed`).
7. Sleep `interval - elapsed` so the cadence does not drift.

The implementation is stateless with respect to the device — it never increments pointers, never writes, never sends preset or control frames. The `Command` enum in `src/protocol/mod.rs` makes the forbidden indices unrepresentable; there is no raw-byte send API.

## Code layout

```
src/
  main.rs                # poll loop, dispatch
  config.rs              # TOML load + 30s floor enforcement
  audit.rs               # daily JSONL append (logs/YYYY-MM-DD.jsonl)
  state.rs               # StateAccumulator + atomic write
  client.rs              # sync TCP, bounded frame read loop
  protocol/{mod,checksum,frame}.rs   # safety-critical core
  parse/{status,clock,device_info,diagnostics,wireless}.rs
```

60 unit tests in-tree. The DT214 canonical query frames and the live 2026-05-26 device_info response are golden fixtures — if any of those tests regress, do not ship.

## Build

- Linux dev: `cargo test && cargo build --release` → `target/release/horustechwatch`
- Windows cross-compile (requires `rustup target add x86_64-pc-windows-gnu` + `apt install mingw-w64`):
  `cargo build --release --target x86_64-pc-windows-gnu` → `target/x86_64-pc-windows-gnu/release/horustechwatch.exe` (~570 KB)

## Configuration (`config.toml`)

- TOML, loaded at startup. Path defaults to `./config.toml`; override via `argv[1]`.
- All paths: use **single-quoted** TOML strings for Windows paths so backslashes are not interpreted as escapes. Example: `state_file = '\\Servidor\pista\Relátório-pista\modulo_alarmes\concentrador\state.json'`.
- The 30-second poll floor and 5-second socket timeout are hardcoded — not overridable via config. The loader rejects `interval_seconds < 30` at startup with a loud error.
