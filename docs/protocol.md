# OpenMSO Capture Protocol (OCP) — version 0 (draft)

License: Apache-2.0. Status: draft — expect breaking changes until v1.

OCP connects a **frontend** (GUI, CLI, scripting environment) to a **capture
plugin** (a separate process that knows how to drive one family of
oscilloscopes / logic analyzers / other sample sources). The frontend launches
the plugin and speaks OCP to it over the plugin's stdin/stdout, or over a TCP
socket. The design borrows deliberately from LSP/DAP: a purpose-built JSON
protocol with capability discovery, easy to implement in any language in an
afternoon, so the ecosystem does not depend on a single repository.

Design decisions, recorded:

- **Not SCPI.** SCPI has no capability discovery, no async event framing, and
  vendor dialects diverge wildly (Siglent `C1:WF? DAT2` vs Rigol
  `:WAV:DATA?`). SCPI remains the *device-side* language inside plugins; a
  `device.raw` passthrough exists for debugging. Devices that natively speak
  SCPI (bench gear, the future Pico MSO firmware) get driven by SCPI-speaking
  plugins.
- **Not protobuf / binary schema.** A schema compiler raises the bar for
  plugin authors — the LSP lesson is that ease of implementation drives
  adoption. Control traffic is human-readable JSON; only bulk samples are raw
  binary (base64 would add 33% and burn CPU at logic-analyzer data rates).
- **stdio first, TCP optional.** stdio ties plugin lifetime to the frontend
  and avoids port management; `--listen` mode serves the same protocol over
  TCP for remote/daemon setups.

## 1. Framing

The transport is a bidirectional byte stream. Each message is a single UTF-8
JSON object on one line, terminated by `\n` (LF). No length headers.

A message MAY carry a binary payload: if the object has a top-level integer
field `"binlen": N`, exactly `N` raw bytes follow the terminating LF and
belong to that message. The next message begins at the byte after the payload.

```
{"jsonrpc":"2.0","method":"capture.data","params":{...},"binlen":1400000}\n
<1400000 raw bytes>
{"jsonrpc":"2.0","method":"capture.end", ...}\n
```

Rules:

- Plugin **stdout is exclusively OCP**. Diagnostics go to stderr or `log`
  notifications. A plugin that prints stray text to stdout is broken.
- Recommended maximum payload per message: 4 MiB (split larger data into
  multiple `capture.data` messages).
- Empty lines are ignored.

## 2. RPC layer

JSON-RPC 2.0. Frontend sends *requests* (`id`, `method`, `params`); plugin
replies with *responses* (`result` or `error {code, message, data?}`). Either
side may send *notifications* (no `id`). Requests are answered in order;
notifications may interleave with responses (e.g. `capture.data` streams while
`acquire.stop` is answered).

Error codes: standard JSON-RPC (-32700 parse, -32601 method not found,
-32602 invalid params) plus plugin-defined codes ≥ 1000
(1000 = device error, 1001 = device disconnected, 1002 = busy,
1003 = unsupported).

## 3. Lifecycle methods (frontend → plugin)

### initialize
First request on the connection.

```json
→ {"protocol_version": 0, "client": {"name": "omso", "version": "0.1"}}
← {"protocol_version": 0,
   "plugin": {"name": "sds1000xe", "version": "0.1", "vendor": "OpenMSO"},
   "capabilities": {"scan": true, "modes": ["single", "snapshot"], "raw": true}}
```

The plugin answers with the highest protocol version ≤ the client's. 

### scan
Probe for devices. `hints` narrows the search (plugin-defined keys;
convention: `address` for network, `serial` for serial numbers).

```json
→ {"hints": {"address": "192.168.1.155"}}
← {"devices": [{"device_id": "tcp:192.168.1.155",
                "vendor": "Siglent Technologies", "model": "SDS1104X-E",
                "serial": "SDSMMGKC6R0663",
                "connection": "tcp://192.168.1.155:5025"}]}
```

### open / close
`open {"device_id": ...}` claims the device; `close {}` releases it. All
device methods below require an open device.

### describe
Returns the channel list and the config schema — the OCP equivalent of
libsigrok's `SR_CONF_*` + `sr_config_list()`.

```json
← {"device": {"vendor": "...", "model": "...", "serial": "..."},
   "channels": [
     {"id": "C1", "kind": "analog", "name": "CH1", "index": 0},
     {"id": "D0", "kind": "logic",  "name": "D0",  "index": 0, "untested": true}
   ],
   "config": {
     "timebase":  {"scope": "device",  "type": "number", "unit": "s/div",
                   "choices": [1e-9, 2e-9, "..."], "get": true, "set": true},
     "vdiv":      {"scope": "analog",  "type": "number", "unit": "V/div",
                   "choices": [5e-4, "..."], "get": true, "set": true},
     "coupling":  {"scope": "analog",  "type": "string",
                   "choices": ["ac", "dc", "gnd"], "get": true, "set": true},
     "trigger":   {"scope": "device",  "type": "object", "get": true, "set": true}
   }}
```

`scope` is `device`, `analog`, or `logic` (the latter two are per-channel:
pass `"channel"` in config calls). Additional conventional keys:
`samplerate` (Sa/s, often get-only), `memory_depth`, `offset` (V),
`probe_factor`, `enabled` (bool, per channel), `limit_samples`, `averaging`,
`logic_threshold` (V).

### config.get / config.set

```json
→ {"keys": ["vdiv", "coupling"], "channel": "C1"}          // config.get
← {"values": {"vdiv": 0.05, "coupling": "dc"}}

→ {"values": {"vdiv": 0.1}, "channel": "C1"}               // config.set
← {"applied": {"vdiv": 0.1}}                                // device-coerced values
```

`config.set` returns what the device actually accepted (devices snap values
to legal steps). Omitting `keys` in `config.get` returns everything readable
in the given scope.

**Trigger value** (the `trigger` config key), simple form:

```json
{"type": "edge", "source": "C1", "slope": "rising", "level": 0.75,
 "position": 0.5}
```

`position` is the trigger point as a fraction of the capture (0..1), like
libsigrok's `HORIZ_TRIGGERPOS`. For logic analyzers a staged form (modeled on
libsigrok's `sr_trigger`) is reserved:
`{"type": "stages", "stages": [[{"channel": "D0", "match": "rising"}, ...]]}`
with match ∈ `zero|one|rising|falling|edge|over|under` (+`value` for
over/under). Plugins advertise which forms they accept in `describe`.

### acquire.start / acquire.stop

```json
→ {"mode": "single"}        // or "snapshot", "continuous"
← {"capture_id": 1}
```

- `single` — arm, wait for trigger, transfer one acquisition.
- `snapshot` — stop the device now and transfer whatever it holds (then
  restore its running state).
- `continuous` — stream acquisitions until `acquire.stop` (optional
  capability).

`acquire.start` returns as soon as acquisition is underway; sample data
arrives as notifications. `acquire.stop` aborts (or ends `continuous`); a
`capture.end` still follows.

### device.raw (optional, for debugging)

```json
→ {"command": "SARA?", "query": true}
← {"response": "1.00E+09Sa/s"}
```

### shutdown
Plugin releases the device and exits after responding. EOF on stdin is
equivalent.

## 4. Capture notifications (plugin → frontend)

A capture is bracketed by `capture.begin` / `capture.end`; between them the
plugin streams `capture.data`. Every message carries the `capture_id`.

### capture.begin

```json
{"capture_id": 1, "samplerate": 1e9, "t0": -0.0007,
 "streams": [
   {"stream": 0, "kind": "analog", "channels": ["C1"], "sample_count": 1400000,
    "encoding": {"dtype": "int8", "scale": 0.002, "offset": 0.0,
                 "unit": "V", "quantity": "voltage", "digits": 3}},
   {"stream": 2, "kind": "logic", "channels": ["D0","D1"], "sample_count": 1400000,
    "encoding": {"unitsize": 1}}
 ]}
```

- `t0`: time of sample 0 relative to the trigger instant, in seconds.
- **Analog encoding**: samples are raw device codes;
  `value = raw * scale + offset` in `unit`. `dtype` ∈ `int8|uint8|int16|
  uint16|float32|float64`, little-endian. Sending raw codes + scale keeps
  transfers small (1 byte/sample from 8-bit scopes) and lossless; the
  frontend applies scaling. `digits` = significant decimal digits (display
  hint, from sigrok's encoding model).
- **Logic encoding**: bit-packed samples, `unitsize` bytes per sample,
  channel *i* of the stream = bit *i* (little-endian across the unit) — 
  byte-identical to libsigrok's `SR_DF_LOGIC`, srzip's `logic-1-*` chunks and
  libsigrokdecode's `srd_session_send()` input, so data flows to files and
  decoders without transformation.

### capture.data

```json
{"jsonrpc":"2.0","method":"capture.data",
 "params": {"capture_id": 1, "stream": 0, "seq": 0,
            "first_sample": 0, "nsamples": 1400000},
 "binlen": 1400000}
<raw bytes>
```

`first_sample` is the absolute sample index of the first sample in the
payload (chunks may arrive in any per-stream order but `seq` increments).

### capture.trigger

`{"capture_id": 1, "sample": 700000}` — position of the trigger in the
sample stream, when known.

### capture.end

`{"capture_id": 1, "ok": true}` or
`{"capture_id": 1, "ok": false, "error": "device timeout"}`.

### event.status / log

`event.status {"state": "armed"|"triggered"|"transferring"|"idle", "detail"}`
for UI feedback; `log {"level": "debug"|"info"|"warning"|"error", "message"}`.

## 5. Mapping to the .sr (srzip) file format

Frontends are encouraged to write sigrok-compatible `.sr` files so captures
open in PulseView: a ZIP containing `version` (`"2"`), `metadata` (INI:
`[device 1]` with `samplerate`, `total probes`/`probe<n>` for logic,
`total analog`/`analog<n>` for analog, `unitsize`, `capturefile = logic-1`),
logic chunks `logic-1-<n>` (bit-packed, exactly OCP logic frames), analog
chunks `analog-1-<ch>-<n>` (float32 native-endian — apply OCP scale/offset
when writing). See `python/openmso/srzip.py`.

## 6. Plugin packaging

A plugin directory contains `plugin.json`:

```json
{"name": "sds1000xe", "run": ["{python}", "plugin.py"],
 "description": "Siglent SDS1000X-E series oscilloscopes"}
```

`run` is the argv to launch the plugin in stdio mode; `{python}` expands to
the frontend's Python interpreter. Plugins should also accept
`--listen <host:port>` to serve TCP instead of stdio.
