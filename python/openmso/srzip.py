# SPDX-License-Identifier: Apache-2.0
"""Independent writer for sigrok's .sr (srzip) session files.

Layout (established from libsigrok's src/output/srzip.c, format version 2):

- ``version``: the ASCII string ``2``.
- ``metadata``: INI. ``[device 1]`` carries ``samplerate`` (human string like
  "1 MHz"), logic channels as ``total probes`` + ``probe<n>`` names (1-based
  by channel index) + ``unitsize`` + ``capturefile = logic-1``, analog
  channels as ``total analog`` + ``analog<n>`` names where numbering starts
  at ``total probes`` + 1.
- Logic chunks ``logic-1-<n>`` (n from 1): bit-packed samples, ``unitsize``
  bytes/sample, channel i = bit i.
- Analog chunks ``analog-1-<ch>-<n>``: native-endian float32 samples.

This module contains no sigrok code; the layout above is interface fact.
"""

import zipfile

import numpy as np


def samplerate_string(rate):
    """Format a samplerate the way sigrok's metadata expects ("1 MHz")."""
    rate = int(round(rate))
    for mult, suffix in ((10**9, "GHz"), (10**6, "MHz"), (10**3, "kHz")):
        if rate >= mult and rate % mult == 0:
            return f"{rate // mult} {suffix}"
    return f"{rate} Hz"


class SrZipWriter:
    """Accumulates capture data and writes a .sr file on close().

    logic_channels: list of names (index = bit position).
    analog_channels: list of names.
    """

    CHUNK_SAMPLES = 4 * 1024 * 1024  # flush analog/logic in ~4 MiB chunks

    def __init__(self, path, samplerate, logic_channels=(), analog_channels=(),
                 unitsize=None):
        self.path = path
        self.samplerate = samplerate
        self.logic_channels = list(logic_channels)
        self.analog_channels = list(analog_channels)
        if self.logic_channels and unitsize is None:
            unitsize = (len(self.logic_channels) + 7) // 8
        self.unitsize = unitsize
        self._logic_chunks = []              # raw bytes
        self._analog_chunks = {i: [] for i in range(len(self.analog_channels))}

    def add_logic(self, data):
        """data: bit-packed bytes, unitsize bytes per sample."""
        self._logic_chunks.append(bytes(data))

    def add_analog(self, channel_index, samples):
        """samples: array-like of floats (already scaled to real units)."""
        arr = np.asarray(samples, dtype=np.float32)
        self._analog_chunks[channel_index].append(arr.tobytes())

    def _metadata(self):
        lines = ["[global]", "sigrok version = 0.5.2 (openmso)", ""]
        lines.append("[device 1]")
        if self.logic_channels:
            lines.append("capturefile = logic-1")
            lines.append(f"total probes = {len(self.logic_channels)}")
        lines.append(f"samplerate = {samplerate_string(self.samplerate)}")
        if self.analog_channels:
            lines.append(f"total analog = {len(self.analog_channels)}")
        if self.logic_channels:
            lines.append(f"unitsize = {self.unitsize}")
            for i, name in enumerate(self.logic_channels):
                lines.append(f"probe{i + 1} = {name}")
        base = len(self.logic_channels)
        for i, name in enumerate(self.analog_channels):
            lines.append(f"analog{base + i + 1} = {name}")
        return "\n".join(lines) + "\n"

    def close(self):
        with zipfile.ZipFile(self.path, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("version", "2")
            z.writestr("metadata", self._metadata())
            if self._logic_chunks:
                blob = b"".join(self._logic_chunks)
                step = self.CHUNK_SAMPLES * self.unitsize
                n = 1
                for off in range(0, len(blob), step):
                    z.writestr(f"logic-1-{n}", blob[off:off + step])
                    n += 1
            base = len(self.logic_channels)
            for i, chunks in self._analog_chunks.items():
                if not chunks:
                    continue
                blob = b"".join(chunks)
                step = self.CHUNK_SAMPLES * 4
                n = 1
                for off in range(0, len(blob), step):
                    z.writestr(f"analog-1-{base + i + 1}-{n}",
                               blob[off:off + step])
                    n += 1

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        if exc[0] is None:
            self.close()
