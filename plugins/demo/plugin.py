#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""OpenMSO demo/simulation plugin.

Synthesizes a mixed-signal capture with no hardware: two analog channels
(sine + square with noise) and eight logic channels (binary counter, plus a
10-bit-frame 115200-baud UART pattern on D7 sending "OpenMSO! "). Useful for
exercising frontends, the protocol, decoders, and file writers.
"""

import math
import os
import sys
import threading

sys.path.insert(0, os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "..", "..", "..", "openmso-api", "python"))

import numpy as np

from openmso.server import CaptureServer, RpcError, INVALID_PARAMS, BUSY

DEFAULTS = {"samplerate": 1_000_000, "sample_count": 100_000,
            "frequency": 1000.0, "amplitude": 1.0, "noise": 0.02}


class DemoPlugin(CaptureServer):
    info = {"name": "demo", "version": "0.1.0", "vendor": "OpenMSO",
            "description": "Simulated mixed-signal device"}
    capabilities = {"scan": True, "modes": ["single"], "raw": False,
                    "trigger_forms": []}

    def __init__(self):
        super().__init__()
        self._open = False
        self._cfg = dict(DEFAULTS)
        self._capture_id = 0
        self._thread = None

    def on_scan(self, params):
        return {"devices": [{"device_id": "demo0", "vendor": "OpenMSO",
                             "model": "Demo MSO", "serial": "DEMO0001",
                             "connection": "demo://0"}]}

    def on_open(self, params):
        self._open = True
        return {}

    def on_close(self, params):
        self._open = False
        return {}

    def on_describe(self, params):
        channels = [
            {"id": "A0", "kind": "analog", "name": "sine", "index": 0},
            {"id": "A1", "kind": "analog", "name": "square", "index": 1},
        ] + [{"id": f"D{i}", "kind": "logic", "name": f"D{i}", "index": i}
             for i in range(8)]
        config = {k: {"scope": "device", "type": "number",
                      "get": True, "set": True} for k in DEFAULTS}
        return {"device": {"vendor": "OpenMSO", "model": "Demo MSO"},
                "channels": channels, "config": config}

    def on_config_get(self, params):
        keys = params.get("keys") or list(self._cfg)
        return {"values": {k: self._cfg[k] for k in keys if k in self._cfg}}

    def on_config_set(self, params):
        applied = {}
        for k, v in (params.get("values") or {}).items():
            if k not in self._cfg:
                raise RpcError(INVALID_PARAMS, f"unknown key {k!r}")
            self._cfg[k] = type(DEFAULTS[k])(v)
            applied[k] = self._cfg[k]
        return {"applied": applied}

    def on_acquire_start(self, params):
        if self._thread is not None and self._thread.is_alive():
            raise RpcError(BUSY, "acquisition already running")
        self._capture_id += 1
        cid = self._capture_id
        self._thread = threading.Thread(target=self._acquire, args=(cid,),
                                        daemon=True)
        self._thread.start()
        return {"capture_id": cid}

    def on_acquire_stop(self, params):
        return {}

    def _acquire(self, cid):
        cfg = self._cfg
        n = int(cfg["sample_count"])
        sr = float(cfg["samplerate"])
        t = np.arange(n) / sr
        f, a = cfg["frequency"], cfg["amplitude"]

        rng = np.random.default_rng(0)
        sine = a * np.sin(2 * math.pi * f * t) + rng.normal(0, cfg["noise"], n)
        square = a * np.sign(np.sin(2 * math.pi * f * t)) \
            + rng.normal(0, cfg["noise"], n)
        # int8 codes, 25 codes per "division" like a real 8-bit scope
        scale = a / 100.0
        sine_codes = np.clip(sine / scale, -127, 127).astype(np.int8)
        square_codes = np.clip(square / scale, -127, 127).astype(np.int8)

        # Logic: D0-D6 = binary counter at f*32; D7 = UART TX 115200-8N1
        counter = (np.arange(n) * (f * 32 / sr)).astype(np.int64) % 128
        logic = counter.astype(np.uint8) & 0x7F
        logic |= (self._uart_bits(n, sr) << 7)

        streams = [
            {"stream": 0, "kind": "analog", "channels": ["A0"],
             "sample_count": n,
             "encoding": {"dtype": "int8", "scale": scale, "offset": 0.0,
                          "unit": "V", "quantity": "voltage", "digits": 3}},
            {"stream": 1, "kind": "analog", "channels": ["A1"],
             "sample_count": n,
             "encoding": {"dtype": "int8", "scale": scale, "offset": 0.0,
                          "unit": "V", "quantity": "voltage", "digits": 3}},
            {"stream": 2, "kind": "logic",
             "channels": [f"D{i}" for i in range(8)], "sample_count": n,
             "encoding": {"unitsize": 1}},
        ]
        self.notify("capture.begin",
                    {"capture_id": cid, "samplerate": sr, "t0": 0.0,
                     "streams": streams})
        self.notify("capture.trigger", {"capture_id": cid, "sample": 0})
        for stream, arr in ((0, sine_codes), (1, square_codes), (2, logic)):
            data = arr.tobytes()
            step = 1 << 20
            for seq, off in enumerate(range(0, len(data), step)):
                part = data[off:off + step]
                self.notify("capture.data",
                            {"capture_id": cid, "stream": stream, "seq": seq,
                             "first_sample": off, "nsamples": len(part)},
                            payload=part)
        self.notify("capture.end", {"capture_id": cid, "ok": True})

    @staticmethod
    def _uart_bits(n, samplerate, baud=115200, text=b"OpenMSO! "):
        """Idle-high async serial bitstream sampled at `samplerate`."""
        bits = []
        for byte in text:
            frame = [0] + [(byte >> i) & 1 for i in range(8)] + [1]
            bits.extend(frame)
        bits.extend([1] * 20)   # inter-message idle
        bits = np.array(bits, dtype=np.uint8)
        idx = (np.arange(n) * (baud / samplerate)).astype(np.int64) % len(bits)
        return bits[idx]


if __name__ == "__main__":
    DemoPlugin().run_from_argv()
