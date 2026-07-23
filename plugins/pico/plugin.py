#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""OpenMSO Pico bridge plugin.

The OpenMSO Pico firmware speaks OCP natively over its USB-CDC serial port, so
this plugin is a thin bridge: it enumerates Pico devices, opens the port, and
forwards OCP requests to the device while relaying its notifications back to the
frontend. Sample bulk arrives in OCP-native encodings (this milestone: none yet
— control only); transport decompression will live here once streaming lands.

This plugin shares no code with the GPLv3 firmware and only speaks the wire
protocol, so it is Apache-2.0 like the rest of the OpenMSO core (see the plan's
§17.3 licensing boundary).
"""

import glob
import os
import queue
import sys
import termios
import threading

sys.path.insert(0, os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "python"))

from openmso.framing import MessageStream, ProtocolError
from openmso.server import (PluginServer, RpcError, INVALID_PARAMS,
                            DEVICE_ERROR, DEVICE_DISCONNECTED)

METHOD_NOT_FOUND = -32601

# by-id symlinks are stable and encode the flash unique ID in their name:
#   usb-OpenMSO_Pico_MSO_E6616407E39C7C2B-if00 -> ../../ttyACM0
BYID_GLOB = "/dev/serial/by-id/*OpenMSO_Pico_MSO_*"


def _decode_edge(payload, nsamples, unitsize):
    """Expand the edge (transition) codec back to flat bit-packed logic.

    The device emits (run_length LEB128, word[unitsize]) records — one per
    change of the logic word. Reconstruct ``nsamples`` samples of ``unitsize``
    bytes each. See plan §8; must mirror ``notify_logic_edge`` in the firmware.
    """
    out = bytearray(nsamples * unitsize)
    pos = 0
    op = 0                       # samples emitted so far
    plen = len(payload)
    while pos < plen and op < nsamples:
        run = 0
        shift = 0
        while True:              # unsigned LEB128
            b = payload[pos]
            pos += 1
            run |= (b & 0x7F) << shift
            if not (b & 0x80):
                break
            shift += 7
        word = bytes(payload[pos:pos + unitsize])
        pos += unitsize
        seg = word * run
        base = op * unitsize
        out[base:base + len(seg)] = seg
        op += run
    return bytes(out)


class DeviceLink:
    """Framed OCP link to one Pico over its CDC serial port.

    A background reader thread routes responses (by id) to waiting callers and
    hands notifications to ``on_notify``.
    """

    def __init__(self, port, on_notify):
        fd = os.open(port, os.O_RDWR | os.O_NOCTTY)
        attr = termios.tcgetattr(fd)
        attr[0] = attr[1] = attr[3] = 0                 # raw iflag/oflag/lflag
        attr[2] = (attr[2] & ~termios.CSIZE) | termios.CS8 \
            | termios.CREAD | termios.CLOCAL
        # Pin a safe line speed: never 1200, which the firmware treats as a
        # reboot-to-BOOTSEL request.
        attr[4] = attr[5] = termios.B115200
        attr[6][termios.VMIN] = 1
        attr[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSANOW, attr)
        termios.tcflush(fd, termios.TCIOFLUSH)

        self._f = os.fdopen(fd, "r+b", buffering=0)
        self._stream = MessageStream(self._f, self._f)
        self._on_notify = on_notify
        self._pending = {}
        self._lock = threading.Lock()
        self._next_id = 1
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()

    def _read_loop(self):
        try:
            while True:
                item = self._stream.read_message()
                if item is None:
                    break
                msg, payload = item
                mid = msg.get("id")
                if mid is not None:
                    with self._lock:
                        slot = self._pending.pop(mid, None)
                    if slot is not None:
                        slot.put((msg, payload))
                elif msg.get("method"):
                    self._on_notify(msg["method"], msg.get("params") or {},
                                    payload)
        except (ProtocolError, OSError):
            pass
        finally:
            with self._lock:
                for slot in self._pending.values():
                    slot.put(None)
                self._pending.clear()

    def request(self, method, params=None, timeout=5.0):
        with self._lock:
            mid = self._next_id
            self._next_id += 1
            slot = queue.Queue(maxsize=1)
            self._pending[mid] = slot
        self._stream.write_message(
            {"jsonrpc": "2.0", "id": mid, "method": method,
             "params": params or {}})
        try:
            item = slot.get(timeout=timeout)
        except queue.Empty:
            with self._lock:
                self._pending.pop(mid, None)
            raise RpcError(DEVICE_ERROR, f"device timeout on {method!r}")
        if item is None:
            raise RpcError(DEVICE_DISCONNECTED, "device disconnected")
        msg, _payload = item
        if "error" in msg:
            err = msg["error"]
            raise RpcError(err.get("code", DEVICE_ERROR),
                           err.get("message", "device error"))
        return msg.get("result") or {}

    def close(self):
        try:
            self._stream.close()
        except OSError:
            pass


class PicoPlugin(PluginServer):
    info = {"name": "pico", "version": "0.1.0", "vendor": "OpenMSO",
            "description": "Raspberry Pi Pico family mixed-signal device"}
    capabilities = {"scan": True, "modes": ["single", "snapshot", "continuous"],
                    "raw": False, "trigger_forms": ["edge"]}

    def __init__(self):
        super().__init__()
        self._link = None
        self._ports = {}   # device_id -> (real_path, serial)

    # -- discovery --------------------------------------------------------
    def _discover(self):
        found = {}
        for link in glob.glob(BYID_GLOB):
            try:
                real = os.path.realpath(link)
            except OSError:
                continue
            serial = link.split("OpenMSO_Pico_MSO_")[-1].split("-if")[0]
            found[f"pico:{serial}"] = (real, serial)
        return found

    def on_scan(self, params):
        self._ports = self._discover()
        return {"devices": [
            {"device_id": did, "vendor": "OpenMSO", "model": "Pico MSO",
             "serial": serial, "connection": f"serial://{real}"}
            for did, (real, serial) in self._ports.items()]}

    # -- lifecycle --------------------------------------------------------
    def on_open(self, params):
        did = params.get("device_id")
        if not self._ports:
            self._ports = self._discover()
        entry = self._ports.get(did)
        if entry is None:  # tolerate a bare serial or path
            entry = next((v for k, v in self._ports.items()
                          if did in (k, v[0], v[1])), None)
        if entry is None:
            raise RpcError(INVALID_PARAMS, f"unknown device {did!r}")
        real, _serial = entry
        self._link = DeviceLink(real, self._forward_notify)
        self._link.request("initialize", {"protocol_version": 0})
        return {}

    def on_close(self, params):
        if self._link:
            self._link.close()
            self._link = None
        return {}

    def _forward_notify(self, method, params, payload):
        # Transport decompression lives here (plan §8): the device may edge-
        # encode logic streams; expand them back to OCP-native flat bit-packed
        # logic so the frontend only ever sees `raw`.
        if method == "capture.data" and params.get("enc") == "edge":
            payload = _decode_edge(payload or b"", params.get("nsamples", 0),
                                   params.get("unitsize", 1))
            params = {k: v for k, v in params.items()
                      if k not in ("enc", "unitsize")}
        try:
            self.notify(method, params, payload)
        except OSError:
            pass

    def _link_or_raise(self):
        if self._link is None:
            raise RpcError(DEVICE_DISCONNECTED, "no device open")
        return self._link

    # -- forwarded device methods ----------------------------------------
    def on_describe(self, params):
        return self._link_or_raise().request("describe", params)

    def on_config_get(self, params):
        try:
            return self._link_or_raise().request("config.get", params)
        except RpcError as e:
            if e.code == METHOD_NOT_FOUND:   # device has no config yet (M0)
                return {"values": {}}
            raise

    def on_config_set(self, params):
        return self._link_or_raise().request("config.set", params)

    def on_acquire_start(self, params):
        # Request edge transport encoding for logic (plan §8); we decode it back
        # to raw in _forward_notify, so it is transparent to the frontend. The
        # device falls back to raw if it doesn't support it.
        params = dict(params or {}, transport_enc="edge")
        return self._link_or_raise().request("acquire.start", params)

    def on_acquire_stop(self, params):
        return self._link_or_raise().request("acquire.stop", params)

    def on_shutdown(self, params):
        if self._link:
            try:
                self._link.request("shutdown", {}, timeout=1.0)
            except RpcError:
                pass
            self._link.close()
            self._link = None
        self._shutdown = True
        return {}

    def on_disconnect(self):
        if self._link:
            self._link.close()
            self._link = None


if __name__ == "__main__":
    PicoPlugin().run_from_argv()
