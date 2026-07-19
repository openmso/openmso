#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""OpenMSO capture plugin for Siglent SDS1000X-E series oscilloscopes.

Written from scratch against the Siglent "Digital Oscilloscope Series
Programming Guide" (EN02E). libsigrok's siglent-sds driver and PR #247 were
consulted as behavioral references only; no GPL code is included.

Verified live on an SDS1104X-E (firmware 8.3.6.1.37R8) over raw TCP :5025.
The digital (D0-D15 / SLA1016) path follows the documentation but has not
been exercised on hardware — channels are advertised with "untested": true.
"""

import glob
import os
import re
import sys
import threading

sys.path.insert(0, os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "python"))

import numpy as np

from openmso.server import (PluginServer, RpcError, DEVICE_ERROR, BUSY,
                            UNSUPPORTED, INVALID_PARAMS)
from openmso.scpi import TcpScpi, UsbTmcScpi, ScpiError, open_transport

VDIVS = [500e-6, 1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3,
         100e-3, 200e-3, 500e-3, 1.0, 2.0, 5.0, 10.0]
TDIVS = [1e-9, 2e-9, 5e-9, 10e-9, 20e-9, 50e-9, 100e-9, 200e-9, 500e-9,
         1e-6, 2e-6, 5e-6, 10e-6, 20e-6, 50e-6, 100e-6, 200e-6, 500e-6,
         1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3, 100e-3, 200e-3, 500e-3,
         1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]
MEMORY_DEPTHS = ["14K", "140K", "1.4M", "14M"]     # interleave-mode values
COUPLING_TO_SCPI = {"ac": "A1M", "dc": "D1M", "gnd": "GND"}
SCPI_TO_COUPLING = {v: k for k, v in COUPLING_TO_SCPI.items()}
HORIZ_DIVS = 14
CODES_PER_DIV = 25
ANALOG_CHANNELS = ["C1", "C2", "C3", "C4"]
LOGIC_CHANNELS = [f"D{i}" for i in range(16)]
# Verified live: a full 14 Mpt (14 MB) block reads fine in one WF?
# transaction at ~6.5 MB/s on both TCP and VXI-11, and paging via WFSU NP/FP
# also works but costs ~35% throughput — so read the whole depth in one shot.
# The paging path below remains for models with deeper memory.
PAGE_SAMPLES = 14_000_363   # SDS1000X-E max buffer
DATA_FRAME_BYTES = 4 * 1024 * 1024

_NUM_RE = re.compile(r"[-+]?[0-9]*\.?[0-9]+(?:[eE][-+]?[0-9]+)?")


def scpi_float(text):
    """Extract the numeric value from a reply like 'SARA 1.00E+09Sa/s'."""
    m = _NUM_RE.search(text.split(",")[-1] if "," in text else text)
    if not m:
        raise ScpiError(f"no number in reply: {text!r}")
    return float(m.group(0))


def fmt_volts(v):
    return f"{v:.6E}V"


class Sds1000XePlugin(PluginServer):
    info = {"name": "sds1000xe", "version": "0.1.0", "vendor": "OpenMSO",
            "description": "Siglent SDS1000X-E series oscilloscopes"}
    capabilities = {"scan": True, "modes": ["single", "snapshot"],
                    "raw": True, "trigger_forms": ["edge"]}

    def __init__(self):
        super().__init__()
        self._dev = None            # ScpiTransport
        self._idn = None
        self._lock = threading.Lock()   # serializes SCPI transactions
        self._capture_id = 0
        self._acq_thread = None
        self._stop_flag = threading.Event()

    # ------------------------------------------------------------------
    # scan / open / close
    # ------------------------------------------------------------------
    def on_scan(self, params):
        hints = params.get("hints") or {}
        devices = []
        addr = hints.get("address")
        if addr:
            host, _, port = addr.partition(":")
            # VXI-11 first: the X-E's raw-socket service (5025) is fragile
            # and once crashed stays down until reboot; VXI-11 is the path
            # vendor software uses.
            probes = [(f"vxi11://{host}", None)] if not port else []
            probes.append((f"tcp://{host}:{port or 5025}", None))
            for conn, _ in probes:
                try:
                    t = open_transport(conn)
                    idn = t.query("*IDN?")
                    t.close()
                    devices.append(self._device_entry(conn, idn))
                    break
                except Exception as e:   # OSError, ScpiError, Vxi11Error
                    self.log("warning", f"scan {conn}: {e}")
        for path in sorted(glob.glob("/dev/usbtmc*")):
            if not os.access(path, os.R_OK | os.W_OK):
                self.log("warning",
                         f"{path}: no permission (install udev rule, see "
                         f"plugins/sds1000xe/99-openmso-usbtmc.rules)")
                continue
            try:
                t = UsbTmcScpi(path)
                idn = t.query("*IDN?")
                t.close()
                if "SDS1" in idn or "SDS2" in idn:
                    devices.append(self._device_entry(f"usbtmc://{path}", idn))
            except (OSError, ScpiError) as e:
                self.log("warning", f"scan {path}: {e}")
        return {"devices": devices}

    @staticmethod
    def _device_entry(connection, idn):
        parts = [p.strip() for p in idn.split(",")]
        vendor, model, serial = (parts + ["?", "?", "?"])[:3]
        return {"device_id": connection, "vendor": vendor, "model": model,
                "serial": serial, "connection": connection,
                "firmware": parts[3] if len(parts) > 3 else None}

    def on_open(self, params):
        if self._dev is not None:
            raise RpcError(BUSY, "device already open")
        connection = params.get("device_id")
        if not connection:
            raise RpcError(INVALID_PARAMS, "device_id required")
        try:
            dev = open_transport(connection)
            idn = dev.query("*IDN?")
            dev.command("CHDR OFF")   # numeric-only replies from here on
        except (OSError, ScpiError) as e:
            raise RpcError(DEVICE_ERROR, f"open failed: {e}")
        self._dev = dev
        self._idn = self._device_entry(connection, idn)
        return {"device": self._idn}

    def on_close(self, params):
        self._release()
        return {}

    def _release(self):
        self._stop_flag.set()
        if self._acq_thread is not None:
            self._acq_thread.join(timeout=10)
            self._acq_thread = None
        if self._dev is not None:
            self._dev.close()
            self._dev = None

    def on_disconnect(self):
        self._release()

    def _require_dev(self):
        if self._dev is None:
            raise RpcError(DEVICE_ERROR, "no device open")
        return self._dev

    def _q(self, cmd):
        with self._lock:
            return self._require_dev().query(cmd)

    def _c(self, cmd):
        with self._lock:
            self._require_dev().command(cmd)

    # ------------------------------------------------------------------
    # describe / config
    # ------------------------------------------------------------------
    def on_describe(self, params):
        self._require_dev()
        channels = [{"id": c, "kind": "analog", "name": f"CH{i + 1}",
                     "index": i} for i, c in enumerate(ANALOG_CHANNELS)]
        channels += [{"id": d, "kind": "logic", "name": d, "index": i,
                      "untested": True} for i, d in enumerate(LOGIC_CHANNELS)]
        config = {
            "timebase": {"scope": "device", "type": "number", "unit": "s/div",
                         "choices": TDIVS, "get": True, "set": True},
            "samplerate": {"scope": "device", "type": "number", "unit": "Sa/s",
                           "get": True, "set": False},
            "memory_depth": {"scope": "device", "type": "string",
                             "choices": MEMORY_DEPTHS, "get": True, "set": True},
            "trigger": {"scope": "device", "type": "object",
                        "get": True, "set": True},
            "enabled": {"scope": "analog", "type": "bool",
                        "get": True, "set": True},
            "vdiv": {"scope": "analog", "type": "number", "unit": "V/div",
                     "choices": VDIVS, "get": True, "set": True},
            "offset": {"scope": "analog", "type": "number", "unit": "V",
                       "get": True, "set": True},
            "coupling": {"scope": "analog", "type": "string",
                         "choices": sorted(COUPLING_TO_SCPI), "get": True,
                         "set": True},
            "probe_factor": {"scope": "analog", "type": "number",
                             "choices": [0.1, 1, 10, 100, 1000],
                             "get": True, "set": True},
        }
        return {"device": self._idn, "channels": channels, "config": config}

    def on_config_get(self, params):
        self._require_dev()
        channel = params.get("channel")
        keys = params.get("keys")
        if channel:
            if channel not in ANALOG_CHANNELS:
                raise RpcError(INVALID_PARAMS, f"unknown channel {channel}")
            getters = {
                "enabled": lambda: self._q(f"{channel}:TRA?").upper().endswith("ON"),
                "vdiv": lambda: scpi_float(self._q(f"{channel}:VDIV?")),
                "offset": lambda: scpi_float(self._q(f"{channel}:OFST?")),
                "coupling": lambda: SCPI_TO_COUPLING.get(
                    self._q(f"{channel}:CPL?").strip(), "dc"),
                "probe_factor": lambda: scpi_float(self._q(f"{channel}:ATTN?")),
            }
        else:
            getters = {
                "timebase": lambda: scpi_float(self._q("TDIV?")),
                "samplerate": lambda: scpi_float(self._q("SARA?")),
                "memory_depth": lambda: self._q("MSIZ?").strip(),
                "trigger": self._trigger_get,
            }
        keys = keys or list(getters)
        values = {}
        for k in keys:
            if k not in getters:
                raise RpcError(INVALID_PARAMS, f"unknown key {k!r} in scope")
            values[k] = getters[k]()
        return {"values": values}

    def on_config_set(self, params):
        self._require_dev()
        channel = params.get("channel")
        values = params.get("values") or {}
        applied = {}
        for k, v in values.items():
            applied[k] = self._set_one(channel, k, v)
        return {"applied": applied}

    def _set_one(self, channel, key, value):
        if channel:
            if channel not in ANALOG_CHANNELS:
                raise RpcError(INVALID_PARAMS, f"unknown channel {channel}")
            if key == "enabled":
                self._c(f"{channel}:TRA {'ON' if value else 'OFF'}")
                return self._q(f"{channel}:TRA?").upper().endswith("ON")
            if key == "vdiv":
                self._c(f"{channel}:VDIV {value:.6E}")
                return scpi_float(self._q(f"{channel}:VDIV?"))
            if key == "offset":
                self._c(f"{channel}:OFST {value:.6E}")
                return scpi_float(self._q(f"{channel}:OFST?"))
            if key == "coupling":
                if value not in COUPLING_TO_SCPI:
                    raise RpcError(INVALID_PARAMS, f"coupling {value!r}")
                self._c(f"{channel}:CPL {COUPLING_TO_SCPI[value]}")
                return SCPI_TO_COUPLING.get(self._q(f"{channel}:CPL?").strip())
            if key == "probe_factor":
                self._c(f"{channel}:ATTN {value:g}")
                return scpi_float(self._q(f"{channel}:ATTN?"))
        else:
            if key == "timebase":
                self._c(f"TDIV {value:.6E}")
                return scpi_float(self._q("TDIV?"))
            if key == "memory_depth":
                if value not in MEMORY_DEPTHS:
                    raise RpcError(INVALID_PARAMS,
                                   f"memory_depth must be one of {MEMORY_DEPTHS}")
                self._c(f"MSIZ {value}")
                return self._q("MSIZ?").strip()
            if key == "trigger":
                return self._trigger_set(value)
        raise RpcError(INVALID_PARAMS, f"unknown key {key!r} in scope")

    def _trigger_get(self):
        trse = self._q("TRSE?")            # e.g. "EDGE,SR,C1,HT,OFF"
        parts = [p.strip() for p in trse.split(",")]
        source = "C1"
        if "SR" in parts:
            source = parts[parts.index("SR") + 1]
        trig = {"type": parts[0].lower() if parts else "edge",
                "source": source}
        if source in ANALOG_CHANNELS or source in ("EX", "EX5"):
            slope = self._q(f"{source}:TRSL?").strip().upper()
            trig["slope"] = {"POS": "rising", "NEG": "falling"}.get(
                slope, slope.lower())
            try:
                trig["level"] = scpi_float(self._q(f"{source}:TRLV?"))
            except ScpiError:
                pass
        mode = self._q("TRMD?").strip().upper()
        trig["mode"] = mode.lower()
        return trig

    def _trigger_set(self, value):
        if not isinstance(value, dict):
            raise RpcError(INVALID_PARAMS, "trigger must be an object")
        if value.get("type", "edge") != "edge":
            raise RpcError(UNSUPPORTED, "only edge trigger supported for now")
        source = value.get("source", "C1")
        if source not in ANALOG_CHANNELS + ["EX", "EX5", "LINE"]:
            raise RpcError(INVALID_PARAMS, f"trigger source {source!r}")
        self._c(f"TRSE EDGE,SR,{source},HT,OFF")
        if "slope" in value:
            scpi_slope = {"rising": "POS", "falling": "NEG"}.get(value["slope"])
            if scpi_slope is None:
                raise RpcError(INVALID_PARAMS, f"slope {value['slope']!r}")
            self._c(f"{source}:TRSL {scpi_slope}")
        if "level" in value and source != "LINE":
            self._c(f"{source}:TRLV {fmt_volts(value['level'])}")
        if "mode" in value:
            mode = value["mode"].upper()
            if mode not in ("AUTO", "NORM", "SINGLE"):
                raise RpcError(INVALID_PARAMS, f"mode {value['mode']!r}")
            self._c(f"TRMD {mode}")
        return self._trigger_get()

    # ------------------------------------------------------------------
    # acquisition
    # ------------------------------------------------------------------
    def on_acquire_start(self, params):
        self._require_dev()
        if self._acq_thread is not None and self._acq_thread.is_alive():
            raise RpcError(BUSY, "acquisition already running")
        mode = params.get("mode", "single")
        if mode not in ("single", "snapshot"):
            raise RpcError(UNSUPPORTED, f"mode {mode!r} not supported")
        timeout = float(params.get("timeout", 30.0))
        self._capture_id += 1
        cid = self._capture_id
        self._stop_flag.clear()
        self._acq_thread = threading.Thread(
            target=self._acquire, args=(cid, mode, timeout), daemon=True)
        self._acq_thread.start()
        return {"capture_id": cid}

    def on_acquire_stop(self, params):
        self._stop_flag.set()
        return {}

    def on_device_raw(self, params):
        cmd = params.get("command", "")
        if params.get("binary"):
            with self._lock:
                data = self._require_dev().query_block(cmd)
            return {"length": len(data)}
        if params.get("query", cmd.rstrip().endswith("?")):
            return {"response": self._q(cmd)}
        self._c(cmd)
        return {}

    # -- worker ---------------------------------------------------------
    def _acquire(self, cid, mode, timeout):
        try:
            self._acquire_inner(cid, mode, timeout)
        except (ScpiError, OSError, RpcError) as e:
            self.notify("capture.end",
                        {"capture_id": cid, "ok": False, "error": str(e)})
        except Exception as e:
            import traceback
            traceback.print_exc(file=sys.stderr)
            self.notify("capture.end",
                        {"capture_id": cid, "ok": False,
                         "error": f"{type(e).__name__}: {e}"})

    def _wait_stopped(self, timeout):
        """Poll SAST? until the scope reports Stop (single-shot complete)."""
        import time
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self._stop_flag.is_set():
                self._c("STOP")
                return True
            state = self._q("SAST?").strip().lower()
            if state == "stop":
                return True
            time.sleep(0.05)
        return False

    def _acquire_inner(self, cid, mode, timeout):
        prev_trmd = self._q("TRMD?").strip().upper()
        if mode == "single":
            self.notify("event.status", {"state": "armed"})
            self._c("TRMD SINGLE")
            if not self._wait_stopped(timeout):
                self._c("STOP")
                raise RpcError(DEVICE_ERROR,
                               f"no trigger within {timeout:.0f}s")
            self.notify("event.status", {"state": "triggered"})
        else:  # snapshot: freeze whatever is on screen
            self._c("STOP")

        enabled = [c for c in ANALOG_CHANNELS
                   if self._q(f"{c}:TRA?").upper().endswith("ON")]
        if not enabled:
            raise RpcError(DEVICE_ERROR, "no channels enabled")
        samplerate = scpi_float(self._q("SARA?"))
        tdiv = scpi_float(self._q("TDIV?"))
        sample_count = int(scpi_float(self._q(f"SANU? {enabled[0]}")))

        streams = []
        chan_meta = {}
        for si, ch in enumerate(enabled):
            vdiv = scpi_float(self._q(f"{ch}:VDIV?"))
            ofst = scpi_float(self._q(f"{ch}:OFST?"))
            chan_meta[ch] = (si, vdiv, ofst)
            streams.append({
                "stream": si, "kind": "analog", "channels": [ch],
                "sample_count": sample_count,
                "encoding": {"dtype": "int8",
                             "scale": vdiv / CODES_PER_DIV,
                             "offset": -ofst, "unit": "V",
                             "quantity": "voltage", "digits": 3}})
        self.notify("capture.begin", {
            "capture_id": cid, "samplerate": samplerate,
            "t0": -(tdiv * HORIZ_DIVS / 2), "timebase": tdiv,
            "streams": streams})

        self.notify("event.status", {"state": "transferring"})
        for ch in enabled:
            si = chan_meta[ch][0]
            self._read_channel(cid, si, ch, sample_count)
            if self._stop_flag.is_set():
                break

        # Restore free-running state and full-transfer setup.
        with self._lock:
            dev = self._require_dev()
            dev.command("WFSU SP,0,NP,0,FP,0")
            if mode == "snapshot" and prev_trmd in ("AUTO", "NORM"):
                dev.command(f"TRMD {prev_trmd}")
        aborted = self._stop_flag.is_set()
        self.notify("capture.end", {"capture_id": cid, "ok": not aborted,
                                    **({"error": "aborted"} if aborted else {})})
        self.notify("event.status", {"state": "idle"})

    def _read_channel(self, cid, stream, ch, sample_count):
        seq = 0
        if sample_count <= PAGE_SAMPLES:
            with self._lock:
                dev = self._require_dev()
                dev.command("WFSU SP,0,NP,0,FP,0")
                data = dev.query_block(f"{ch}:WF? DAT2", timeout=30.0)
            data = data[:sample_count]
            for off in range(0, len(data), DATA_FRAME_BYTES):
                part = data[off:off + DATA_FRAME_BYTES]
                self.notify("capture.data",
                            {"capture_id": cid, "stream": stream, "seq": seq,
                             "first_sample": off, "nsamples": len(part)},
                            payload=part)
                seq += 1
            return
        first = 0
        while first < sample_count and not self._stop_flag.is_set():
            npoints = min(PAGE_SAMPLES, sample_count - first)
            with self._lock:
                dev = self._require_dev()
                dev.command(f"WFSU SP,0,NP,{npoints},FP,{first}")
                data = dev.query_block(f"{ch}:WF? DAT2", timeout=30.0)
            if not data:
                raise ScpiError(f"empty page at FP={first} for {ch}")
            data = data[:npoints]
            self.notify("capture.data",
                        {"capture_id": cid, "stream": stream, "seq": seq,
                         "first_sample": first, "nsamples": len(data)},
                        payload=data)
            seq += 1
            first += len(data)


if __name__ == "__main__":
    Sds1000XePlugin().run_from_argv()
