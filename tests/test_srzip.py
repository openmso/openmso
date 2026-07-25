# SPDX-License-Identifier: Apache-2.0
"""Unit tests: srzip writer layout.

OCP framing is tested in the openmso-api repo (framing lives there now).
"""

import os
import sys
import zipfile

import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python"))

from srzip import SrZipWriter, samplerate_string


def test_samplerate_string():
    assert samplerate_string(1e9) == "1 GHz"
    assert samplerate_string(500e6) == "500 MHz"
    assert samplerate_string(1e3) == "1 kHz"
    assert samplerate_string(12500) == "12500 Hz"


def test_srzip_layout(tmp_path):
    path = str(tmp_path / "t.sr")
    w = SrZipWriter(path, 1_000_000, logic_channels=["D0", "D1"],
                    analog_channels=["C1"])
    w.add_logic(b"\x00\x01\x02\x03")
    w.add_analog(0, np.array([0.0, 1.5, -1.5], dtype=np.float32))
    w.close()

    with zipfile.ZipFile(path) as z:
        names = set(z.namelist())
        assert {"version", "metadata", "logic-1-1", "analog-1-3-1"} <= names
        assert z.read("version") == b"2"
        meta = z.read("metadata").decode()
        assert "total probes = 2" in meta
        assert "samplerate = 1 MHz" in meta
        assert "unitsize = 1" in meta
        assert "probe1 = D0" in meta
        assert "analog3 = C1" in meta          # numbering starts after probes
        assert z.read("logic-1-1") == b"\x00\x01\x02\x03"
        vals = np.frombuffer(z.read("analog-1-3-1"), dtype=np.float32)
        assert list(vals) == [0.0, 1.5, -1.5]


def test_srzip_analog_only(tmp_path):
    path = str(tmp_path / "a.sr")
    w = SrZipWriter(path, 1_000_000_000, analog_channels=["C1", "C3"])
    w.add_analog(0, np.zeros(10, dtype=np.float32))
    w.add_analog(1, np.ones(10, dtype=np.float32))
    w.close()
    with zipfile.ZipFile(path) as z:
        meta = z.read("metadata").decode()
        assert "total analog = 2" in meta
        assert "capturefile" not in meta
        assert "analog1 = C1" in meta and "analog2 = C3" in meta
        assert {"analog-1-1-1", "analog-1-2-1"} <= set(z.namelist())
